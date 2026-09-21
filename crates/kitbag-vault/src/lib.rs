//! Where the values live.
//!
//! kitbag does not implement a secret store. It borrows one, so a person keeps
//! using what they already trust — and so that losing interest in kitbag does
//! not strand their secrets inside it.
//!
//! A backend only has to keep **bytes under a name**. Everything kitbag needs
//! to know about an item — its scope, its owner, the hash of its payload —
//! travels inside [`kitbag_core::Envelope`], so adding a backend never means
//! teaching it what a scope is. A backend that has native metadata may mirror
//! the header into it so its own UI shows the scope; that mirror is a
//! convenience, never the source of truth.
//!
//! | backend | store | notes |
//! | --- | --- | --- |
//! | [`BackendKind::Bw`] | Bitwarden / Vaultwarden | free tier, self-hostable, attachments for large payloads |
//! | [`BackendKind::Op`] | 1Password | the best developer CLI in the category; `op://vault/item/field` |
//! | [`BackendKind::Pass`] | `pass` / `gopass` (GPG) | a tree of encrypted files; no metadata of its own, which the envelope makes irrelevant |
//! | [`BackendKind::Age`] | an `age`-encrypted file | no server at all; can sit beside a public repo |

pub mod agefile;
pub mod bw;
pub mod shellout;

use std::collections::BTreeMap;
use std::str::FromStr;
use std::sync::Mutex;

use anyhow::{bail, Result};
use kitbag_core::Envelope;

/// What a store can do beyond holding bytes. kitbag works with all of these
/// false; they only let it choose a better path when one exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Capabilities {
    /// Native metadata that can mirror the envelope header (Bitwarden custom
    /// fields, 1Password fields). Cosmetic: the envelope is authoritative.
    pub fields: bool,
    /// Somewhere to put a large payload other than the note body.
    pub attachments: bool,
    /// Largest note the store accepts, if it has a limit worth knowing about.
    pub max_note_bytes: Option<usize>,
}

/// What a store reports about an item without being asked for its value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listing {
    pub name: String,
    /// The payload alone, for comparing against a file on disk. Present when
    /// the backend can report it cheaply; otherwise kitbag reads the envelope.
    pub payload_hash: Option<String>,
    /// The payload and every header over it, for deciding whether a push has
    /// anything to say. A marker that changed moves this and not the other.
    pub fingerprint: Option<String>,
    /// Whose it is, when the store can say without being asked for the value.
    /// A restore takes only the scopes a machine asked for, and deciding that
    /// from the listing is the difference between fetching everything to find
    /// out and fetching what is actually wanted.
    pub scope: Option<kitbag_core::Scope>,
    /// The platforms the item belongs on, when the store can say without being
    /// asked for the value. `None` is "cannot say cheaply", not "everywhere".
    pub platform: Option<Vec<String>>,
    /// The machine an item belongs to, when the store can say cheaply.
    pub machine: Option<String>,
}

pub trait Backend {
    fn capabilities(&self) -> Capabilities;

    /// Every item this store holds for kitbag — names and hashes, never values.
    /// One call, so comparing a machine against a store is cheap.
    fn list(&self) -> Result<Vec<Listing>>;

    /// One item. Callers hold the result as briefly as they can.
    fn get(&self, name: &str) -> Result<Envelope>;

    /// Create or update.
    ///
    /// There is deliberately no `delete`. A store holds things this machine
    /// knows nothing about — another machine's key, an account someone else
    /// added — and a tool that removes what it does not recognise eventually
    /// removes something that mattered. Removal is a person's decision, taken
    /// with the store's own client.
    fn put(&self, name: &str, envelope: &Envelope) -> Result<()>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendKind {
    Bw,
    Op,
    Pass,
    Age,
    /// In-process, for tests. Never touches a disk or a network.
    Memory,
}

impl FromStr for BackendKind {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        Ok(match s.trim().to_ascii_lowercase().as_str() {
            "bw" | "bitwarden" | "vaultwarden" => BackendKind::Bw,
            "op" | "1password" | "onepassword" => BackendKind::Op,
            "pass" | "gopass" | "gpg" => BackendKind::Pass,
            "age" | "file" => BackendKind::Age,
            "memory" => BackendKind::Memory,
            other => bail!("unknown backend `{other}` (expected bw, op, pass, age)"),
        })
    }
}

impl BackendKind {
    /// The store named, or the one a machine would use by default.
    pub fn from_str_or_default(name: Option<&str>) -> Result<Self> {
        name.unwrap_or("bw").parse()
    }

    pub fn open(self) -> Result<Box<dyn Backend>> {
        match self {
            BackendKind::Memory => Ok(Box::new(MemoryBackend::default())),
            BackendKind::Bw => Ok(Box::new(bw::Bw::new()?)),
            BackendKind::Op => Ok(Box::new(shellout::Op::new()?)),
            BackendKind::Pass => Ok(Box::new(shellout::Pass::new()?)),
            BackendKind::Age => Ok(Box::new(agefile::AgeFile::new()?)),
        }
    }
}

/// A store that keeps everything in memory, with no metadata of its own — the
/// least capable backend there can be. Anything that works against this works
/// against `pass`, and anything that needs more than this is a bug in kitbag,
/// not a missing feature in somebody's password manager.
#[derive(Default)]
pub struct MemoryBackend {
    items: Mutex<BTreeMap<String, String>>,
}

impl MemoryBackend {
    pub fn len(&self) -> usize {
        self.items.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The raw text a store would hold — what a person would see in their vault.
    pub fn raw(&self, name: &str) -> Option<String> {
        self.items.lock().unwrap().get(name).cloned()
    }
}

impl Backend for MemoryBackend {
    fn capabilities(&self) -> Capabilities {
        Capabilities::default()
    }

    fn list(&self) -> Result<Vec<Listing>> {
        Ok(self
            .items
            .lock()
            .unwrap()
            .iter()
            .map(|(name, text)| {
                let parsed = Envelope::parse(text).ok();
                Listing {
                    name: name.clone(),
                    payload_hash: parsed.as_ref().map(|e| e.sha256()),
                    fingerprint: parsed.as_ref().map(|e| e.fingerprint()),
                    platform: parsed.as_ref().map(|e| e.platform.clone()),
                    machine: parsed.as_ref().and_then(|e| e.machine.clone()),
                    scope: parsed.map(|e| e.scope),
                }
            })
            .collect())
    }

    fn get(&self, name: &str) -> Result<Envelope> {
        let items = self.items.lock().unwrap();
        let Some(text) = items.get(name) else {
            bail!("no such item: {name}");
        };
        Ok(Envelope::parse(text)?)
    }

    fn put(&self, name: &str, envelope: &Envelope) -> Result<()> {
        self.items
            .lock()
            .unwrap()
            .insert(name.to_string(), envelope.to_text());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kitbag_core::Scope;

    fn env(scope: Scope, body: &str) -> Envelope {
        Envelope::new(scope, body.as_bytes().to_vec())
    }

    #[test]
    fn a_store_with_no_metadata_still_keeps_the_scope() {
        let b = MemoryBackend::default();
        b.put(
            "env:github",
            &env(Scope::Work, "TOKEN=x").with_owner(Some("acme".into())),
        )
        .unwrap();

        let got = b.get("env:github").unwrap();
        assert_eq!(got.scope, Scope::Work);
        assert_eq!(got.owner.as_deref(), Some("acme"));
        assert_eq!(got.payload, b"TOKEN=x");
    }

    #[test]
    fn listing_reports_hashes_without_handing_out_values() {
        let b = MemoryBackend::default();
        b.put("env:a", &env(Scope::Personal, "one")).unwrap();
        b.put("env:b", &env(Scope::Personal, "two")).unwrap();

        let list = b.list().unwrap();
        assert_eq!(list.len(), 2);
        assert!(list.iter().all(|l| l.payload_hash.is_some()));
        // The hash is of the payload, so an unchanged item compares equal
        // without anybody fetching it.
        assert_eq!(
            list.iter()
                .find(|l| l.name == "env:a")
                .unwrap()
                .payload_hash,
            Some(env(Scope::Personal, "one").sha256())
        );
    }

    #[test]
    fn putting_twice_updates_rather_than_duplicates() {
        let b = MemoryBackend::default();
        b.put("env:a", &env(Scope::Personal, "one")).unwrap();
        b.put("env:a", &env(Scope::Personal, "two")).unwrap();
        assert_eq!(b.len(), 1);
        assert_eq!(b.get("env:a").unwrap().payload, b"two");
    }

    #[test]
    fn binary_goes_through_a_text_only_store() {
        let b = MemoryBackend::default();
        let bytes = vec![0u8, 1, 2, 250, 255];
        b.put("file:keychain", &Envelope::new(Scope::Work, bytes.clone()))
            .unwrap();
        assert!(b.raw("file:keychain").unwrap().is_ascii());
        assert_eq!(b.get("file:keychain").unwrap().payload, bytes);
    }

    #[test]
    fn backends_are_named_the_way_people_say_them() {
        for (s, want) in [
            ("bw", BackendKind::Bw),
            ("vaultwarden", BackendKind::Bw),
            ("1password", BackendKind::Op),
            ("gpg", BackendKind::Pass),
            ("gopass", BackendKind::Pass),
            ("age", BackendKind::Age),
        ] {
            assert_eq!(BackendKind::from_str(s).unwrap(), want, "for {s}");
        }
        assert!(BackendKind::from_str("lastpass").is_err());
    }

    #[test]
    fn every_backend_is_written_even_where_none_is_reachable() {
        // Each may fail to open here - no client installed, nothing unlocked -
        // but none may fail because nobody wrote it.
        for kind in [
            BackendKind::Bw,
            BackendKind::Op,
            BackendKind::Pass,
            BackendKind::Age,
        ] {
            if let Err(err) = kind.open() {
                assert!(
                    !err.to_string().contains("not implemented"),
                    "{kind:?}: {err}"
                );
            }
        }
    }
}
