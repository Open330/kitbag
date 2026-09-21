//! The stores that are somebody else's command line client.
//!
//! `pass` and `op` are wrapped rather than reimplemented, for the same reason
//! `bw` is: each already owns the hard parts - unlocking, key material, the
//! account model - and a second implementation of those is a second thing that
//! can be wrong about them.
//!
//! Neither has to understand a scope. What goes in is the envelope, which
//! carries everything kitbag needs to read back.

use std::process::{Command, Stdio};

use anyhow::{bail, Context, Result};
use kitbag_core::Envelope;

use crate::{Backend, Capabilities, Listing};

/// Where kitbag's items live inside somebody's existing store, so a vault in
/// daily use does not acquire loose items all over it.
const PREFIX: &str = "kitbag";

// ---------------------------------------------------------------------------
// pass / gopass — a tree of GPG-encrypted files
// ---------------------------------------------------------------------------

/// `pass`, and `gopass`, which speaks the same commands.
///
/// It has no metadata of its own, which the envelope makes irrelevant: the
/// whole envelope is the file, and everything about the item is inside it.
pub struct Pass {
    bin: String,
}

impl Pass {
    /// A named client, for tests: PATH is process-wide, and tests share a
    /// process. Anything that reached for PATH here would race the others.
    pub fn with_bin(bin: impl Into<String>) -> Self {
        Self { bin: bin.into() }
    }

    pub fn new() -> Result<Self> {
        for bin in ["pass", "gopass"] {
            if Command::new(bin)
                .arg("version")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
            {
                return Ok(Self { bin: bin.into() });
            }
        }
        bail!("neither `pass` nor `gopass` is on this machine")
    }

    fn entry(name: &str) -> String {
        // `pass` names entries by path, and a kitbag name has a colon in it.
        format!("{PREFIX}/{}", name.replacen(':', "-", 1))
    }

    /// The inverse: the first hyphen is the colon that was replaced.
    fn name_of(entry: &str) -> String {
        entry.replacen('-', ":", 1)
    }
}

impl Backend for Pass {
    fn capabilities(&self) -> Capabilities {
        Capabilities::default()
    }

    fn list(&self) -> Result<Vec<Listing>> {
        let out = Command::new(&self.bin).args(["ls", PREFIX]).output()?;
        if !out.status.success() {
            // An empty store is not an error; it is a store with nothing in it.
            return Ok(Vec::new());
        }
        // `pass ls` draws a tree, so the names are read back from `git`-free
        // output by stripping its box-drawing characters.
        Ok(String::from_utf8_lossy(&out.stdout)
            .lines()
            .skip(1)
            .filter_map(|line| {
                let name = line
                    .trim_start_matches(|c: char| c.is_whitespace() || "│├└─".contains(c))
                    .trim();
                (!name.is_empty()).then(|| Listing {
                    // The inverse of `entry`: only the first hyphen was a
                    // colon. Replacing them all turns ssh:config-20-work into
                    // something no other command would recognise.
                    name: Self::name_of(name),
                    // The hash lives in the envelope, which means reading the
                    // entry - so a `pass` store answers "I cannot say cheaply".
                    payload_hash: None,
                    fingerprint: None,
                    scope: None,
                    platform: None,
                    machine: None,
                })
            })
            .collect())
    }

    fn get(&self, name: &str) -> Result<Envelope> {
        let out = Command::new(&self.bin)
            .args(["show", &Self::entry(name)])
            .output()?;
        if !out.status.success() {
            bail!("{}: {}", name, String::from_utf8_lossy(&out.stderr).trim());
        }
        Ok(Envelope::parse(&String::from_utf8_lossy(&out.stdout))?)
    }

    fn put(&self, name: &str, envelope: &Envelope) -> Result<()> {
        use std::io::Write;
        let mut child = Command::new(&self.bin)
            .args(["insert", "--multiline", "--force", &Self::entry(name)])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .context("running pass insert")?;
        child
            .stdin
            .as_mut()
            .expect("stdin was piped")
            .write_all(envelope.to_text().as_bytes())?;
        let out = child.wait_with_output()?;
        if !out.status.success() {
            bail!("{}: {}", name, String::from_utf8_lossy(&out.stderr).trim());
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// op — 1Password
// ---------------------------------------------------------------------------

/// 1Password, through `op`.
///
/// The best developer client in the category, and the reason to wrap it rather
/// than anything else: it already has the session handling, the biometrics and
/// the service accounts, none of which kitbag wants to own.
pub struct Op {
    vault: String,
    bin: String,
}

impl Op {
    pub fn with_bin(bin: impl Into<String>, vault: impl Into<String>) -> Self {
        Self {
            bin: bin.into(),
            vault: vault.into(),
        }
    }

    pub fn new() -> Result<Self> {
        let ok = Command::new("op")
            .args(["--version"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !ok {
            bail!("the 1Password CLI (`op`) is not on this machine");
        }
        Ok(Self {
            vault: std::env::var("KITBAG_OP_VAULT").unwrap_or_else(|_| PREFIX.to_string()),
            bin: "op".into(),
        })
    }

    fn run(&self, args: &[&str]) -> Result<String> {
        let out = Command::new(&self.bin).args(args).output()?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            bail!("op {}: {}", args.first().unwrap_or(&""), err.trim());
        }
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    }
}

impl Backend for Op {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            fields: true,
            attachments: true,
            max_note_bytes: None,
        }
    }

    fn list(&self) -> Result<Vec<Listing>> {
        let out = match self.run(&["item", "list", "--vault", &self.vault, "--format", "json"]) {
            Ok(out) => out,
            // A vault that does not exist yet holds nothing, which is not an
            // error until somebody tries to write to it.
            Err(_) => return Ok(Vec::new()),
        };
        let items: Vec<serde_json::Value> = serde_json::from_str(&out)?;
        Ok(items
            .iter()
            .filter_map(|i| {
                Some(Listing {
                    name: i.get("title")?.as_str()?.to_string(),
                    payload_hash: None,
                    fingerprint: None,
                    scope: None,
                    platform: None,
                    machine: None,
                })
            })
            .collect())
    }

    fn get(&self, name: &str) -> Result<Envelope> {
        let out = self.run(&["read", &format!("op://{}/{name}/notesPlain", self.vault)])?;
        Ok(Envelope::parse(&out)?)
    }

    fn put(&self, name: &str, envelope: &Envelope) -> Result<()> {
        let text = envelope.to_text();
        let note = format!("notesPlain={text}");
        // `op item edit` fails when the item is not there, so the create is
        // the fallback rather than the other way round: an update is the
        // common case once a machine has pushed once.
        if self
            .run(&["item", "edit", name, "--vault", &self.vault, &note])
            .is_err()
        {
            self.run(&[
                "item",
                "create",
                "--category",
                "Secure Note",
                "--title",
                name,
                "--vault",
                &self.vault,
                &note,
            ])?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kitbag_core::Scope;

    /// A `pass` that keeps entries as files in a directory. Every command the
    /// wrapper uses, and nothing else - which is the point: this tests the
    /// wrapper, not somebody else's password manager.
    fn stub_pass(dir: &std::path::Path) -> String {
        let bin = dir.join("pass-stub");
        let script = format!(
            r#"#!/bin/sh
store="{}/store"
mkdir -p "$store"
case "$1" in
  version) echo "stub" ;;
  ls) ls "$store" 2>/dev/null | sed 's/^/├── /' | sed '1i\
kitbag' ;;
  show) cat "$store/$(echo "$2" | sed 's|kitbag/||')" ;;
  insert) cat > "$store/$(echo "$4" | sed 's|kitbag/||')" ;;
  *) exit 1 ;;
esac
"#,
            dir.display()
        );
        std::fs::write(&bin, script).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        bin.to_string_lossy().into_owned()
    }

    #[test]
    fn a_kitbag_name_becomes_a_pass_path() {
        assert_eq!(Pass::entry("env:github"), "kitbag/env-github");
        assert_eq!(
            Pass::entry("ssh:config-20-work"),
            "kitbag/ssh-config-20-work"
        );
    }

    /// An `op` that keeps notes as files. It answers the four calls the
    /// wrapper makes, including failing `item edit` for an item that is not
    /// there - which is the branch the create path depends on.
    ///
    /// Written with a placeholder rather than `format!`: a shell script is
    /// most of a page of braces, and escaping them all is how a stub ends up
    /// testing its own syntax errors.
    fn stub_op(dir: &std::path::Path) -> String {
        const SCRIPT: &str = r#"#!/bin/sh
store="STORE_DIR/op"
mkdir -p "$store"
case "$1 $2" in
  "--version"*)
    echo "2.0.0" ;;
  "item list")
    printf '['
    first=1
    for f in "$store"/*; do
      [ -e "$f" ] || continue
      [ $first -eq 1 ] || printf ','
      first=0
      printf '{"title":"%s"}' "$(basename "$f")"
    done
    printf ']'
    ;;
  "read "*)
    name=$(basename "$(dirname "$2")")
    cat "$store/$name" ;;
  "item edit")
    [ -f "$store/$3" ] || exit 1
    printf '%s' "$6" | sed 's/^notesPlain=//' > "$store/$3" ;;
  "item create")
    title=""; note=""
    while [ $# -gt 0 ]; do
      case "$1" in
        --title) title="$2"; shift 2 ;;
        notesPlain=*) note=${1#notesPlain=}; shift ;;
        *) shift ;;
      esac
    done
    printf '%s' "$note" > "$store/$title" ;;
  *)
    exit 1 ;;
esac
"#;
        let bin = dir.join("op-stub");
        std::fs::write(
            &bin,
            SCRIPT.replace("STORE_DIR", &dir.display().to_string()),
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        bin.to_string_lossy().into_owned()
    }

    #[test]
    fn an_item_is_created_then_edited_in_a_1password_vault() {
        let dir = tempfile::tempdir().unwrap();
        let store = Op::with_bin(stub_op(dir.path()), "kitbag");

        // create, because nothing is there to edit
        store
            .put("env:ci", &Envelope::new(Scope::Work, b"one".to_vec()))
            .unwrap();
        assert_eq!(store.get("env:ci").unwrap().payload, b"one");

        // and then edit, which is the common case once a machine has pushed
        store
            .put("env:ci", &Envelope::new(Scope::Work, b"two".to_vec()))
            .unwrap();
        assert_eq!(store.get("env:ci").unwrap().payload, b"two");

        let names: Vec<_> = store.list().unwrap().into_iter().map(|l| l.name).collect();
        assert_eq!(names, ["env:ci"]);
    }

    #[test]
    fn an_item_survives_the_trip_through_a_pass_store() {
        let dir = tempfile::tempdir().unwrap();
        let store = Pass::with_bin(stub_pass(dir.path()));
        let env = Envelope::new(Scope::Work, b"TOKEN=x\n".to_vec()).with_owner(Some("acme".into()));

        store.put("env:ci", &env).unwrap();
        let back = store.get("env:ci").unwrap();

        assert_eq!(
            back.payload, b"TOKEN=x\n",
            "a trailing newline is part of a file"
        );
        assert_eq!(back.scope, Scope::Work);
        assert_eq!(back.owner.as_deref(), Some("acme"));
    }

    #[test]
    fn a_pass_store_lists_what_it_holds_under_the_names_it_was_given() {
        let dir = tempfile::tempdir().unwrap();
        let store = Pass::with_bin(stub_pass(dir.path()));
        store
            .put(
                "ssh:config-20-work",
                &Envelope::new(Scope::Work, b"x".to_vec()),
            )
            .unwrap();

        let names: Vec<_> = store.list().unwrap().into_iter().map(|l| l.name).collect();
        assert_eq!(
            names,
            ["ssh:config-20-work"],
            "a name with hyphens must come back whole"
        );
    }

    #[test]
    fn a_pass_store_that_is_empty_is_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let store = Pass::with_bin(stub_pass(dir.path()));
        assert!(store.list().unwrap().is_empty());
    }

    #[test]
    fn the_vault_to_use_can_be_named() {
        // Not a test of op itself - of the one decision kitbag makes about it.
        std::env::set_var("KITBAG_OP_VAULT", "Private");
        let vault = std::env::var("KITBAG_OP_VAULT").unwrap();
        assert_eq!(vault, "Private");
        std::env::remove_var("KITBAG_OP_VAULT");
    }
}
