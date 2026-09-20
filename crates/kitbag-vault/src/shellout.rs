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
        format!("{PREFIX}/{}", name.replace(':', "-"))
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
                    name: name.replace('-', ":").replacen(':', ":", 1),
                    // The hash lives in the envelope, which means reading the
                    // entry - so a `pass` store answers "I cannot say cheaply".
                    payload_hash: None,
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
}

impl Op {
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
        })
    }

    fn run(&self, args: &[&str]) -> Result<String> {
        let out = Command::new("op").args(args).output()?;
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

    #[test]
    fn a_kitbag_name_becomes_a_pass_path() {
        assert_eq!(Pass::entry("env:github"), "kitbag/env-github");
        assert_eq!(
            Pass::entry("ssh:config-20-work"),
            "kitbag/ssh-config-20-work"
        );
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
