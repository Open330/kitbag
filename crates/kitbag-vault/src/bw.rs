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

use anyhow::{anyhow, bail, Context, Result};
use kitbag_core::Envelope;

use crate::{Backend, Capabilities, Listing};

/// The folder every kitbag item lives under, so a vault someone already uses
/// does not acquire loose items all over it.
const FOLDER: &str = "kitbag";

pub struct Bw {
    session: Option<String>,
}

impl Bw {
    /// A session is not created here. `bw` owns unlocking - it prompts, it
    /// holds the master password, it decides what counts as unlocked - and a
    /// second thing asking for that password would be a second thing that
    /// could get it wrong.
    pub fn new() -> Result<Self> {
        let status = run(&["status"], None, None)?;
        let parsed: serde_json::Value = serde_json::from_str(&status)
            .context("bw status did not return JSON; is this the Bitwarden CLI?")?;

        match parsed.get("status").and_then(|s| s.as_str()) {
            Some("unlocked") => Ok(Self {
                session: std::env::var("BW_SESSION").ok(),
            }),
            Some("locked") => bail!("the vault is locked — run `bw unlock` and export BW_SESSION"),
            Some("unauthenticated") => bail!("not logged in — run `bw login` first"),
            other => bail!("bw reports an unfamiliar status: {other:?}"),
        }
    }

    fn call(&self, args: &[&str], stdin: Option<&[u8]>) -> Result<String> {
        run(args, self.session.as_deref(), stdin)
    }

    fn items(&self) -> Result<Vec<serde_json::Value>> {
        let out = self.call(&["list", "items"], None)?;
        let all: Vec<serde_json::Value> = serde_json::from_str(&out)?;
        Ok(all
            .into_iter()
            .filter(|i| field(i, "kitbag").is_some())
            .collect())
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
        if let Some(id) = self.folder_id()? {
            return Ok(id);
        }
        let body = serde_json::json!({ "name": FOLDER }).to_string();
        let encoded = self.call(&["encode"], Some(body.as_bytes()))?;
        let created = self.call(&["create", "folder", encoded.trim()], None)?;
        let value: serde_json::Value = serde_json::from_str(&created)?;
        value
            .get("id")
            .and_then(|i| i.as_str())
            .map(str::to_string)
            .ok_or_else(|| anyhow!("bw created a folder without an id"))
    }
}

impl Backend for Bw {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            fields: true,
            attachments: true,
            // Notes are capped well below this, but the envelope base64-encodes
            // binary, so the limit that matters is the encoded size.
            max_note_bytes: Some(10_000),
        }
    }

    fn list(&self) -> Result<Vec<Listing>> {
        Ok(self
            .items()?
            .iter()
            .filter_map(|item| {
                let name = item.get("name")?.as_str()?.to_string();
                Some(Listing {
                    name,
                    payload_hash: field(item, "hash"),
                })
            })
            .collect())
    }

    fn get(&self, name: &str) -> Result<Envelope> {
        let item = self
            .items()?
            .into_iter()
            .find(|i| i.get("name").and_then(|n| n.as_str()) == Some(name))
            .ok_or_else(|| anyhow!("no such item in the vault: {name}"))?;

        let notes = item
            .get("notes")
            .and_then(|n| n.as_str())
            .ok_or_else(|| anyhow!("{name} has no notes to read"))?;
        Ok(Envelope::parse(notes)?)
    }

    fn put(&self, name: &str, envelope: &Envelope) -> Result<()> {
        let folder = self.ensure_folder()?;
        let existing = self
            .items()?
            .into_iter()
            .find(|i| i.get("name").and_then(|n| n.as_str()) == Some(name));

        let fields = serde_json::json!([
            { "name": "kitbag", "value": "1", "type": 0 },
            { "name": "scope", "value": envelope.scope.name(), "type": 0 },
            { "name": "owner", "value": envelope.owner.clone().unwrap_or_default(), "type": 0 },
            { "name": "hash", "value": envelope.sha256(), "type": 0 },
        ]);

        let body = serde_json::json!({
            "type": 2,
            "name": name,
            "notes": envelope.to_text(),
            "folderId": folder,
            "secureNote": { "type": 0 },
            "fields": fields,
            "login": null, "card": null, "identity": null,
        });

        let encoded = self.call(&["encode"], Some(body.to_string().as_bytes()))?;
        match existing.and_then(|i| i.get("id").and_then(|v| v.as_str()).map(str::to_string)) {
            Some(id) => self.call(&["edit", "item", &id, encoded.trim()], None)?,
            None => self.call(&["create", "item", encoded.trim()], None)?,
        };
        Ok(())
    }
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

fn run(args: &[&str], session: Option<&str>, stdin: Option<&[u8]>) -> Result<String> {
    let mut cmd = Command::new("bw");
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

fn first_line(s: &str) -> String {
    s.lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("no message")
        .trim()
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

    #[test]
    fn an_error_is_reported_by_its_first_useful_line() {
        assert_eq!(first_line("\n\n  boom  \nand then some\n"), "boom");
        assert_eq!(first_line(""), "no message");
    }
}
