//! A store that is one encrypted file and no server at all.
//!
//! This is the backend that makes kitbag usable by somebody who has no vault:
//! `age` encrypts to a keypair, the file can live in a private repository, on
//! a USB stick, in any sync folder, and it keeps working when a hosted service
//! does not — including the one you were going to use to get back in.
//!
//! The whole store is a single file holding one envelope per item. That is a
//! deliberate limit: it keeps the format inspectable and the code small, and
//! this is not the backend for someone with ten thousand secrets.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use age::secrecy::ExposeSecret;
use anyhow::{bail, Context, Result};
use kitbag_core::Envelope;

use crate::{Backend, Capabilities, Listing};

pub struct AgeFile {
    store: PathBuf,
    identity: age::x25519::Identity,
}

impl AgeFile {
    /// The store and the key both default under `~/.config/kitbag`, and the
    /// key is created on first use — a backend nobody can start using is not
    /// serving the person who has no vault.
    pub fn new() -> Result<Self> {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default();
        let dir = std::env::var_os("KITBAG_AGE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config/kitbag"));
        Self::at(&dir)
    }

    /// A store in a named directory. Tests use this rather than the
    /// environment: tests share one process, and a variable set in one thread
    /// is set for all of them.
    pub fn at(dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(dir)?;

        let key_path = dir.join("age.key");
        let identity = if key_path.exists() {
            let text = std::fs::read_to_string(&key_path)?;
            text.trim()
                .parse::<age::x25519::Identity>()
                .map_err(|e| anyhow::anyhow!("{key_path:?}: {e}"))?
        } else {
            let identity = age::x25519::Identity::generate();
            write_private(&key_path, identity.to_string().expose_secret())?;
            eprintln!("  a new age key is at {}", key_path.display());
            eprintln!("  back it up: without it this store cannot be opened again.");
            identity
        };

        Ok(Self {
            store: dir.join("store.age"),
            identity,
        })
    }

    /// The decrypted store is JSON: name to envelope.
    ///
    /// It was a separator-delimited format first, and a round trip found the
    /// bug within a minute - the separator's leading newline ate the payload's
    /// trailing one, so every hash came back wrong. JSON has one escaping
    /// problem and it is already solved.
    fn read_all(&self) -> Result<BTreeMap<String, String>> {
        if !self.store.exists() {
            return Ok(BTreeMap::new());
        }
        let encrypted = std::fs::read(&self.store)?;
        let decryptor = age::Decryptor::new(&encrypted[..]).context("reading the store")?;
        let mut reader = decryptor
            .decrypt(std::iter::once(&self.identity as &dyn age::Identity))
            .context("the key in this directory does not open this store")?;
        let mut plain = String::new();
        reader.read_to_string(&mut plain)?;
        if plain.trim().is_empty() {
            return Ok(BTreeMap::new());
        }
        Ok(serde_json::from_str(&plain)
            .context("the store is not in a shape this version reads")?)
    }

    fn write_all(&self, items: &BTreeMap<String, String>) -> Result<()> {
        let plain = serde_json::to_string_pretty(items)?;

        let recipient = self.identity.to_public();
        let encryptor =
            age::Encryptor::with_recipients(std::iter::once(&recipient as &dyn age::Recipient))
                .context("building the encryptor")?;
        let mut encrypted = Vec::new();
        let mut writer = encryptor.wrap_output(&mut encrypted)?;
        writer.write_all(plain.as_bytes())?;
        writer.finish()?;

        // Written beside and moved into place: a store half-written is a store
        // that cannot be opened, and this is the only copy.
        let temp = self.store.with_extension("age.new");
        write_private(&temp, "")?;
        std::fs::write(&temp, &encrypted)?;
        std::fs::rename(&temp, &self.store)?;
        Ok(())
    }
}

fn write_private(path: &Path, contents: &str) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, contents)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(())
}

impl Backend for AgeFile {
    fn capabilities(&self) -> Capabilities {
        Capabilities::default()
    }

    fn list(&self) -> Result<Vec<Listing>> {
        Ok(self
            .read_all()?
            .into_iter()
            .map(|(name, text)| Listing {
                payload_hash: Envelope::parse(&text).ok().map(|e| e.sha256()),
                name,
            })
            .collect())
    }

    fn get(&self, name: &str) -> Result<Envelope> {
        let items = self.read_all()?;
        let Some(text) = items.get(name) else {
            bail!("no such item in the store: {name}");
        };
        Ok(Envelope::parse(text)?)
    }

    fn put(&self, name: &str, envelope: &Envelope) -> Result<()> {
        let mut items = self.read_all()?;
        items.insert(name.to_string(), envelope.to_text());
        self.write_all(&items)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kitbag_core::Scope;

    fn store() -> (tempfile::TempDir, AgeFile) {
        let dir = tempfile::tempdir().unwrap();
        let store = AgeFile::at(dir.path()).unwrap();
        (dir, store)
    }

    #[test]
    fn an_item_survives_a_round_trip_through_the_file() {
        let (_dir, store) = store();
        let env = Envelope::new(Scope::Work, b"TOKEN=x".to_vec()).with_owner(Some("acme".into()));

        store.put("env:ci", &env).unwrap();
        let back = store.get("env:ci").unwrap();

        assert_eq!(back.payload, b"TOKEN=x");
        assert_eq!(back.scope, Scope::Work);
        assert_eq!(back.owner.as_deref(), Some("acme"));
    }

    #[test]
    fn what_lands_on_disk_is_encrypted_and_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let (dir, store) = store();
        store
            .put(
                "env:a",
                &Envelope::new(Scope::Personal, b"SECRET=hunter2".to_vec()),
            )
            .unwrap();

        let raw = std::fs::read(dir.path().join("store.age")).unwrap();
        let as_text = String::from_utf8_lossy(&raw);
        assert!(!as_text.contains("hunter2"), "the payload is in the clear");
        assert!(as_text.starts_with("age-encryption.org"), "not an age file");

        let key_mode = std::fs::metadata(dir.path().join("age.key"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(key_mode & 0o777, 0o600);
    }

    #[test]
    fn several_items_coexist_and_one_can_be_replaced() {
        let (_dir, store) = store();
        store
            .put("env:a", &Envelope::new(Scope::Personal, b"one".to_vec()))
            .unwrap();
        store
            .put("env:b", &Envelope::new(Scope::Personal, b"two".to_vec()))
            .unwrap();
        store
            .put(
                "env:a",
                &Envelope::new(Scope::Personal, b"one, revised".to_vec()),
            )
            .unwrap();

        let names: Vec<_> = store.list().unwrap().into_iter().map(|l| l.name).collect();
        assert_eq!(names, ["env:a", "env:b"]);
        assert_eq!(store.get("env:a").unwrap().payload, b"one, revised");
        assert_eq!(store.get("env:b").unwrap().payload, b"two");
    }

    #[test]
    fn listing_reports_hashes_so_an_unchanged_item_is_never_rewritten() {
        let (_dir, store) = store();
        let env = Envelope::new(Scope::Personal, b"x".to_vec());
        store.put("env:a", &env).unwrap();

        let listing = store.list().unwrap();
        assert_eq!(listing[0].payload_hash, Some(env.sha256()));
    }

    #[test]
    fn a_store_written_by_another_key_will_not_open() {
        let (dir, store) = store();
        store
            .put("env:a", &Envelope::new(Scope::Personal, b"x".to_vec()))
            .unwrap();

        // Replace the key with a different one, as a restore from the wrong
        // backup would.
        let other = age::x25519::Identity::generate();
        write_private(
            &dir.path().join("age.key"),
            other.to_string().expose_secret(),
        )
        .unwrap();
        let reopened = AgeFile::at(dir.path()).unwrap();

        let err = reopened.list().unwrap_err().to_string();
        assert!(err.contains("does not open"), "{err}");
    }

    #[test]
    fn an_empty_store_is_not_an_error() {
        let (_dir, store) = store();
        assert!(store.list().unwrap().is_empty());
    }
}
