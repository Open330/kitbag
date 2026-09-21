//! Bitwarden and Vaultwarden, through the `bw` command line client.
//!
//! Wrapping the client rather than speaking the protocol is a deliberate
//! choice, not a shortcut. Implementing Bitwarden's crypto means writing
//! cipher code against a vault that holds everything its owner has, and the
//! speed argument for it mostly went away once unchanged items stopped being
//! rewritten: a quiet push is one listing plus the changes.
//!
//! What kitbag stores is its own envelope (see [`kitbag_core::Envelope`]), so
//! nothing here needs to know what a scope is. The envelope goes in the note;
//! the scope and owner are mirrored into custom fields as a courtesy to
//! whoever opens the vault in a browser, and are never read back from there.

use std::process::{Command, Stdio};
use std::sync::Mutex;

use anyhow::{anyhow, bail, Context, Result};
use base64::Engine as _;
use kitbag_core::Envelope;

use crate::{Backend, Capabilities, Listing};

/// The folder every kitbag item lives under, so a vault someone already uses
/// does not acquire loose items all over it.
const FOLDER: &str = "kitbag";

/// Bitwarden refuses a note longer than this once encrypted. An envelope
/// base64-encodes anything that is not text, so the limit is reached by
/// ordinary things: an app's exported bundle, a keychain, a tarball.
const MAX_NOTE: usize = 10_000;

/// Where the payload goes when the note will not hold it.
const ATTACHMENT: &str = "kitbag.envelope";

/// What the note says instead. It is deliberately readable: someone opening
/// this item in a browser should be able to tell what happened to the value
/// without knowing anything about kitbag.
const ELSEWHERE: &str = concat!(
    "kitbag/1 elsewhere\n",
    "attachment: kitbag.envelope\n",
    "\n",
    "This payload is larger than a note may be, so it is stored as the\n",
    "attachment named above. `kitbag restore` reads it from there.\n",
);

fn stored_elsewhere(notes: &str) -> bool {
    notes.starts_with("kitbag/1 elsewhere")
}

pub struct Bw {
    session: Option<String>,
    /// `bw list items` decrypts the whole vault, and it returns every item in
    /// full — notes included. Asking once and keeping the answer is the
    /// difference between one decryption per run and one per item: restoring
    /// thirty-nine items used to cost thirty-nine of them.
    items: Mutex<Option<Vec<serde_json::Value>>>,
    folder: Mutex<Option<String>>,
    /// Items this run wrote whose cached copy was dropped, and the id to
    /// re-read them by if anything asks.
    stale: Mutex<std::collections::HashMap<String, String>>,
}

impl Bw {
    /// A session is not created here. `bw` owns unlocking - it prompts, it
    /// holds the master password, it decides what counts as unlocked - and a
    /// second thing asking for that password would be a second thing that
    /// could get it wrong.
    ///
    /// Nor is `bw status` asked here. Every call to this client is a node
    /// process costing over a second before it does anything, and a check that
    /// passes on every successful run is a second spent on every successful
    /// run. A locked vault makes the first real call fail, and that failure is
    /// where the question gets asked.
    pub fn new() -> Result<Self> {
        Ok(Self {
            session: std::env::var("BW_SESSION").ok(),
            items: Mutex::new(None),
            folder: Mutex::new(None),
            stale: Mutex::new(std::collections::HashMap::new()),
        })
    }

    fn call(&self, args: &[&str], stdin: Option<&[u8]>) -> Result<String> {
        run(args, self.session.as_deref(), stdin).map_err(|e| self.explain(e))
    }

    /// Turn whatever the client said into the thing to do about it. Only
    /// reached when something already failed, so the extra call costs nothing
    /// on a run that works.
    fn explain(&self, failure: anyhow::Error) -> anyhow::Error {
        let Ok(status) = run(&["status"], self.session.as_deref(), None) else {
            return failure;
        };
        let parsed: serde_json::Value = match serde_json::from_str(&status) {
            Ok(v) => v,
            Err(_) => return anyhow!("bw status did not return JSON; is this the Bitwarden CLI?"),
        };
        match parsed.get("status").and_then(|s| s.as_str()) {
            Some("locked") => {
                anyhow!("the vault is locked — run `bw unlock` and export BW_SESSION")
            }
            Some("unauthenticated") => anyhow!("not logged in — run `bw login` first"),
            _ => failure,
        }
    }

    /// The vault's kitbag items, fetched at most once. Borrowed rather than
    /// cloned: the callers want one item out of it, not a copy of all of them.
    fn with_items<T>(&self, f: impl FnOnce(&[serde_json::Value]) -> T) -> Result<T> {
        let mut held = self.items.lock().unwrap_or_else(|e| e.into_inner());
        if held.is_none() {
            // `bw list items` reads the client's own copy of the vault and does
            // not go to the server for it. Without a sync this compares a
            // machine against whatever that copy last happened to hold — which
            // makes an item look unchanged when the store does not have it in
            // that form, and a backup that skips an item because of a stale
            // cache is the failure this whole thing exists to avoid.
            //
            // It costs about five seconds, once per run, and it is the reason
            // the shell engine has always opened with "Syncing vault".
            self.call(&["sync"], None)?;
            let out = self.call(&["list", "items"], None)?;
            // A session that expired mid-run leaves the client exiting happily
            // with nothing on stdout. Reporting that as a JSON parse error at
            // line 1 column 0 tells nobody anything.
            let all: Vec<serde_json::Value> = serde_json::from_str(&out)
                .map_err(|e| self.explain(anyhow!("bw list items said nothing usable: {e}")))?;
            *held = Some(
                all.into_iter()
                    .filter(|i| field(i, "kitbag").is_some())
                    .collect(),
            );
        }
        Ok(f(held.as_deref().unwrap_or_default()))
    }

    fn item_named(&self, name: &str) -> Result<Option<serde_json::Value>> {
        // Marked behind by a write this run made: re-read the one item rather
        // than the vault, and only because something actually asked.
        let behind = self
            .stale
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(name);
        if let Some(id) = behind {
            let fresh: serde_json::Value =
                serde_json::from_str(&self.call(&["get", "item", &id], None)?)?;
            self.remember(fresh.clone());
            return Ok(Some(fresh));
        }
        self.with_items(|all| {
            all.iter()
                .find(|i| i.get("name").and_then(|n| n.as_str()) == Some(name))
                .cloned()
        })
    }

    /// Note that this item's cached copy is behind, and where to get a fresh
    /// one if anybody asks. Nobody usually does: a push writes each item once
    /// and never reads it back, so the call this defers is a call that is
    /// never made.
    /// Note that this item's attachment list is behind, and where to get a
    /// fresh one. What was written is cached as written, so a listing in the
    /// same run still sees the item; only the part this does not know — the id
    /// bw assigned the new attachment — is marked for re-reading. Nobody
    /// usually reads it: a push writes each item once, so the call this defers
    /// is a call that is never made.
    fn stale(&self, item: serde_json::Value, name: &str, id: &str) {
        self.remember(item);
        self.stale
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(name.to_string(), id.to_string());
    }

    /// Keep the cache true after a write, rather than dropping it: a push
    /// writes item after item, and a cache thrown away each time is no cache.
    fn remember(&self, item: serde_json::Value) {
        let name = item.get("name").and_then(|n| n.as_str()).unwrap_or("");
        let mut held = self.items.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(all) = held.as_mut() {
            match all
                .iter()
                .position(|i| i.get("name").and_then(|n| n.as_str()) == Some(name))
            {
                Some(at) => all[at] = item,
                None => all.push(item),
            }
        }
    }

    fn folder_id(&self) -> Result<Option<String>> {
        let out = self.call(&["list", "folders"], None)?;
        let folders: Vec<serde_json::Value> = serde_json::from_str(&out)?;
        Ok(folders
            .iter()
            .find(|f| f.get("name").and_then(|n| n.as_str()) == Some(FOLDER))
            .and_then(|f| f.get("id").and_then(|i| i.as_str()))
            .map(str::to_string))
    }

    fn ensure_folder(&self) -> Result<String> {
        if let Some(id) = self
            .folder
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
        {
            return Ok(id);
        }
        // Every item already listed says which folder it is in, and they are
        // all in this one. Asking bw for the folder list is a second process
        // spent learning what is in hand.
        if let Some(id) = self.with_items(|all| {
            all.iter().find_map(|i| {
                i.get("folderId")
                    .and_then(|f| f.as_str())
                    .map(str::to_string)
            })
        })? {
            *self.folder.lock().unwrap_or_else(|e| e.into_inner()) = Some(id.clone());
            return Ok(id);
        }
        if let Some(id) = self.folder_id()? {
            *self.folder.lock().unwrap_or_else(|e| e.into_inner()) = Some(id.clone());
            return Ok(id);
        }
        let body = serde_json::json!({ "name": FOLDER }).to_string();
        let created = self.call(&["create", "folder", &encode(body.as_bytes())], None)?;
        let value: serde_json::Value = serde_json::from_str(&created)?;
        let id = value
            .get("id")
            .and_then(|i| i.as_str())
            .map(str::to_string)
            .ok_or_else(|| anyhow!("bw created a folder without an id"))?;
        *self.folder.lock().unwrap_or_else(|e| e.into_inner()) = Some(id.clone());
        Ok(id)
    }
}

impl Backend for Bw {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            fields: true,
            attachments: true,
            // The envelope base64-encodes binary, so the limit that matters is
            // the encoded size. Past it, `put` uses the attachment above.
            max_note_bytes: Some(MAX_NOTE),
        }
    }

    fn list(&self) -> Result<Vec<Listing>> {
        self.with_items(|all| {
            all.iter()
                .filter_map(|item| {
                    let name = item.get("name")?.as_str()?.to_string();
                    let notes = item.get("notes").and_then(|n| n.as_str()).unwrap_or("");
                    Some(Listing {
                        name,
                        payload_hash: field(item, "hash"),
                        // An inline note is exactly the envelope text, so its
                        // fingerprint is free. A payload that went to an
                        // attachment leaves the field behind instead.
                        fingerprint: if stored_elsewhere(notes) || notes.is_empty() {
                            field(item, "fingerprint")
                        } else {
                            Some(kitbag_core::payload_hash(notes.as_bytes()))
                        },
                        // Written by `put` as a courtesy to whoever opens the
                        // vault in a browser; it costs nothing to read back,
                        // and the envelope stays the authority if they differ.
                        scope: field(item, "scope").and_then(|s| s.parse().ok()),
                        platform: field(item, "platform")
                            .map(|p| p.split(',').map(|s| s.trim().to_string()).collect()),
                        machine: field(item, "machine"),
                    })
                })
                .collect()
        })
    }

    fn get(&self, name: &str) -> Result<Envelope> {
        let item = self
            .item_named(name)?
            .ok_or_else(|| anyhow!("no such item in the vault: {name}"))?;

        let notes = item
            .get("notes")
            .and_then(|n| n.as_str())
            .ok_or_else(|| anyhow!("{name} has no notes to read"))?;

        if !stored_elsewhere(notes) {
            return Ok(Envelope::parse(notes)?);
        }

        let id = item
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("{name} has an attachment but no id to fetch it by"))?;

        // `bw get attachment` writes a file and will not write to a pipe, so
        // the payload touches disk. It touches a directory this process owns
        // and that goes with it, which is the same bargain the shell engine
        // made for exactly this reason.
        let dir = tempfile::Builder::new()
            .prefix("kitbag-")
            .tempdir()
            .context("nowhere to receive the attachment")?;
        let out = dir.path().join(ATTACHMENT);
        self.call(
            &[
                "get",
                "attachment",
                ATTACHMENT,
                "--itemid",
                id,
                "--output",
                &out.to_string_lossy(),
            ],
            None,
        )?;
        let text = std::fs::read_to_string(&out)
            .with_context(|| format!("{name}: the attachment did not arrive"))?;
        Ok(Envelope::parse(&text)?)
    }

    fn put(&self, name: &str, envelope: &Envelope) -> Result<()> {
        let folder = self.ensure_folder()?;
        let existing = self.item_named(name)?;

        // Read the limit off the declared capability rather than the constant,
        // so the two cannot drift into disagreeing about what this store does.
        let text = envelope.to_text();
        let inline = text.len() <= self.capabilities().max_note_bytes.unwrap_or(usize::MAX);
        let body = item_body(name, envelope, &folder, inline);

        // `bw encode` is base64 and nothing else — verified against it — and
        // every call to the client is a node process. Thirty-nine items is
        // thirty-nine of them, spent on an encoding this can do itself.
        let encoded = encode(body.to_string().as_bytes());
        let id = match existing
            .as_ref()
            .and_then(|i| i.get("id").and_then(|v| v.as_str()))
        {
            Some(id) => {
                self.call(&["edit", "item", id, &encoded], None)?;
                id.to_string()
            }
            None => {
                let created = self.call(&["create", "item", &encoded], None)?;
                let value: serde_json::Value = serde_json::from_str(&created)?;
                value
                    .get("id")
                    .and_then(|i| i.as_str())
                    .map(str::to_string)
                    .ok_or_else(|| anyhow!("bw created an item without an id"))?
            }
        };

        if inline {
            // What the cache should now say about this item: what was sent,
            // under the id it was sent to, keeping whatever attachments it
            // already had.
            let mut written = body;
            written["id"] = serde_json::Value::String(id);
            written["attachments"] = existing
                .as_ref()
                .and_then(|i| i.get("attachments").cloned())
                .unwrap_or(serde_json::Value::Array(Vec::new()));
            self.remember(written);
            return Ok(());
        }

        let stale_id = id.clone();

        // Which attachments were already here, so the ones replaced can go and
        // nothing that arrived later is mistaken for them.
        let replaced = existing.as_ref().map(attachment_ids).unwrap_or_default();

        let dir = tempfile::Builder::new()
            .prefix("kitbag-")
            .tempdir()
            .context("nowhere to stage the attachment")?;
        let staged = dir.path().join(ATTACHMENT);
        std::fs::write(&staged, text.as_bytes())
            .with_context(|| format!("{name}: could not stage the payload"))?;

        // The new copy goes up before the old one comes down: a failure in
        // between leaves two payloads on the item, which `get` resolves, and
        // never leaves it with none.
        self.call(
            &[
                "create",
                "attachment",
                "--itemid",
                &id,
                "--file",
                &staged.to_string_lossy(),
            ],
            None,
        )?;

        // This is the one deletion in the file, and it is not the deletion the
        // trait refuses: it removes a copy of this item's own payload that this
        // tool wrote and has just superseded, never an item, and never anything
        // it did not put there itself.
        //
        // And it cannot fail the write. The new copy is already up — that is
        // the whole reason it goes first — so failing here reports an item as
        // not sent when it was, leaves the agreement unrecorded, and sends it
        // again next time to fail in the same place. Two of these machines sat
        // stuck on one item for that reason, one because the attachment had
        // already gone and one because the client threw.
        for old in replaced {
            if let Err(e) = self.call(&["delete", "attachment", &old, "--itemid", &id], None) {
                eprintln!(
                    "  kitbag: {name} is up to date, but an old copy of its \
                     attachment could not be removed: {}",
                    first_line(&e.to_string())
                );
            }
        }

        // The new attachment's id is bw's to assign, and caching a guess at it
        // would have this tool delete the wrong one next time. So what is known
        // is cached, and the rest is marked for re-reading if anything asks.
        let mut written = body;
        written["id"] = serde_json::Value::String(id);
        self.stale(written, name, &stale_id);
        Ok(())
    }
}

/// The item as bw wants it. Pure, so what goes into the vault can be asserted
/// on without a vault.
fn item_body(name: &str, envelope: &Envelope, folder: &str, inline: bool) -> serde_json::Value {
    let fields = serde_json::json!([
        { "name": "kitbag", "value": "1", "type": 0 },
        { "name": "scope", "value": envelope.scope.name(), "type": 0 },
        { "name": "owner", "value": envelope.owner.clone().unwrap_or_default(), "type": 0 },
        { "name": "hash", "value": envelope.sha256(), "type": 0 },
        { "name": "platform", "value": envelope.platform.join(", "), "type": 0 },
        { "name": "machine", "value": envelope.machine.clone().unwrap_or_default(), "type": 0 },
        { "name": "fingerprint", "value": envelope.fingerprint(), "type": 0 },
    ]);

    serde_json::json!({
        "type": 2,
        "name": name,
        // The hash lives in a field either way, so a listing still compares
        // without fetching whatever the note does or does not hold.
        "notes": if inline { envelope.to_text() } else { ELSEWHERE.to_string() },
        "folderId": folder,
        "secureNote": { "type": 0 },
        "fields": fields,
        "login": null, "card": null, "identity": null,
    })
}

/// The ids of the payload attachments already on an item.
fn attachment_ids(item: &serde_json::Value) -> Vec<String> {
    item.get("attachments")
        .and_then(|a| a.as_array())
        .map(|all| {
            all.iter()
                .filter(|a| a.get("fileName").and_then(|f| f.as_str()) == Some(ATTACHMENT))
                .filter_map(|a| a.get("id").and_then(|i| i.as_str()).map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// What `bw encode` does, without the process.
fn encode(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn field(item: &serde_json::Value, name: &str) -> Option<String> {
    item.get("fields")?
        .as_array()?
        .iter()
        .find(|f| f.get("name").and_then(|n| n.as_str()) == Some(name))
        .and_then(|f| f.get("value").and_then(|v| v.as_str()))
        .map(str::to_string)
        .filter(|v| !v.is_empty())
}

/// The client to run. Overridable so the argument shapes this file depends on
/// can be tested without a vault: what `bw` is asked for is checkable, even
/// though what `bw` does with it is not.
fn client() -> String {
    std::env::var("KITBAG_BW").unwrap_or_else(|_| "bw".to_string())
}

fn run(args: &[&str], session: Option<&str>, stdin: Option<&[u8]>) -> Result<String> {
    let mut cmd = Command::new(client());
    cmd.args(args);
    if let Some(s) = session {
        cmd.env("BW_SESSION", s);
    }
    cmd.stdin(if stdin.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    });
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

    let mut child = cmd
        .spawn()
        .context("could not run `bw` — is the Bitwarden CLI installed?")?;
    if let Some(bytes) = stdin {
        use std::io::Write;
        child
            .stdin
            .as_mut()
            .ok_or_else(|| anyhow!("bw would not take input"))?
            .write_all(bytes)?;
    }
    let out = child.wait_with_output()?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        bail!("bw {}: {}", args.first().unwrap_or(&""), first_line(&err));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// The line of a client's stderr that says what went wrong.
///
/// Taking the first non-empty one gives the right answer for a message and
/// the wrong one for a crash: node prints the file and line it threw at
/// first, so a run reported
///
/// ```text
/// Error: bw get: /…/@bitwarden/cli/build/bw.js:53662
/// ```
///
/// and kept the reason to itself. A thrown error names its kind before its
/// message — `TypeError: …` — so that is what to look for, and the first
/// non-empty line is what to fall back on.
fn first_line(s: &str) -> String {
    let named = s.lines().map(str::trim).find(|l| {
        l.split_once(": ").is_some_and(|(kind, rest)| {
            kind.ends_with("Error")
                && !kind.contains('/')
                && kind.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                && !rest.is_empty()
        })
    });
    if let Some(line) = named {
        return line.to_string();
    }
    s.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("no message")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_custom_field_is_read_by_name_and_an_empty_one_counts_as_absent() {
        let item = serde_json::json!({
            "fields": [
                { "name": "kitbag", "value": "1" },
                { "name": "owner", "value": "" },
            ]
        });
        assert_eq!(field(&item, "kitbag").as_deref(), Some("1"));
        assert_eq!(field(&item, "owner"), None, "an empty owner is no owner");
        assert_eq!(field(&item, "nothing"), None);
    }

    #[test]
    fn an_item_with_no_fields_at_all_does_not_panic() {
        assert_eq!(field(&serde_json::json!({ "name": "x" }), "kitbag"), None);
    }

    fn envelope_of(len: usize) -> Envelope {
        Envelope::new(kitbag_core::Scope::Personal, vec![b'x'; len])
    }

    #[test]
    fn a_payload_a_note_can_hold_goes_in_the_note() {
        let env = envelope_of(16);
        let body = item_body("env:small", &env, "folder-id", true);
        let notes = body["notes"].as_str().unwrap();
        assert!(
            notes.starts_with("kitbag/1"),
            "the envelope itself is there"
        );
        assert!(!stored_elsewhere(notes));
    }

    #[test]
    fn a_payload_too_big_for_a_note_leaves_a_pointer_instead() {
        // This is the case that stopped a push dead: an app bundle encodes to
        // more than a Bitwarden note may hold.
        let env = envelope_of(MAX_NOTE * 2);
        assert!(env.to_text().len() > MAX_NOTE);

        let body = item_body("app:big", &env, "folder-id", false);
        let notes = body["notes"].as_str().unwrap();
        assert!(stored_elsewhere(notes));
        assert!(notes.len() < MAX_NOTE, "a pointer is small by construction");
        assert!(
            notes.contains(ATTACHMENT),
            "it names where the payload went"
        );
    }

    #[test]
    fn the_hash_is_a_field_whichever_way_the_payload_went() {
        // `list` reads the hash off the field, so an item whose payload is an
        // attachment still compares without anything being downloaded.
        let env = envelope_of(MAX_NOTE * 2);
        for inline in [true, false] {
            let body = item_body("app:big", &env, "folder-id", inline);
            let hash = body["fields"]
                .as_array()
                .unwrap()
                .iter()
                .find(|f| f["name"] == "hash")
                .unwrap()["value"]
                .as_str()
                .unwrap()
                .to_string();
            assert_eq!(hash, env.sha256(), "inline = {inline}");
        }
    }

    #[test]
    fn only_the_payload_attachment_this_tool_wrote_is_replaced() {
        let item = serde_json::json!({
            "attachments": [
                { "id": "a1", "fileName": ATTACHMENT },
                { "id": "a2", "fileName": "notes-from-someone.pdf" },
            ]
        });
        assert_eq!(attachment_ids(&item), vec!["a1"], "nothing else is touched");
    }

    #[test]
    fn an_item_with_no_attachments_replaces_nothing() {
        assert!(attachment_ids(&serde_json::json!({ "name": "x" })).is_empty());
        assert!(attachment_ids(&serde_json::json!({ "attachments": [] })).is_empty());
    }

    #[test]
    fn an_error_is_reported_by_its_first_useful_line() {
        assert_eq!(first_line("\n\n  boom  \nand then some\n"), "boom");
        assert_eq!(first_line(""), "no message");
    }

    #[test]
    fn a_crash_is_reported_by_what_it_says_not_where_it_happened() {
        // What a real run produced, and what it left out.
        // The path is stubbed: a real one names a person, and this file is
        // linted for exactly that.
        let crash = "/…/node_modules/\
                     @bitwarden/cli/build/bw.js:53662\n\
                     \x20           throw new Error(t);\n\
                     \x20           ^\n\n\
                     TypeError: e.getSingleMessage is not a function\n\
                     \x20   at x (/…/bw.js:53662:19)\n";
        assert_eq!(
            first_line(crash),
            "TypeError: e.getSingleMessage is not a function"
        );
    }

    #[test]
    fn a_plain_message_is_still_the_plain_message() {
        // The client's own errors are not thrown, and must not be skipped past.
        assert_eq!(
            first_line("Attachment `abc` was not found.\n"),
            "Attachment `abc` was not found."
        );
        assert_eq!(
            first_line("The client copy of this cipher is out of date.\n"),
            "The client copy of this cipher is out of date."
        );
    }
}
