//! What this machine and the store last agreed on.
//!
//! Without it a difference has no direction. `status` could say an item
//! differed from the store and no more, so moving a machine across meant
//! deciding by hand for every one of them — including the ones where only one
//! side had moved and there was nothing to decide.
//!
//! This is the missing third point: the fingerprint at the last successful
//! exchange, which is what git calls the merge base. With it, a difference
//! sorts itself:
//!
//! | this machine | the store | |
//! |---|---|---|
//! | moved | still at the base | send it |
//! | still at the base | moved | take it |
//! | moved | moved | a conflict, and a person's to settle |
//!
//! It holds hashes and names. Not values, and not enough to reconstruct one.

use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Default, Clone)]
pub struct Ledger {
    entries: BTreeMap<String, String>,
}

impl Ledger {
    /// A missing file is an empty ledger, not an error: a machine that has
    /// never exchanged anything has nothing to have recorded.
    pub fn load(path: &Path) -> Self {
        let Ok(text) = std::fs::read_to_string(path) else {
            return Self::default();
        };
        let mut entries = BTreeMap::new();
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((name, print)) = line.split_once('\t') {
                entries.insert(name.trim().to_string(), print.trim().to_string());
            }
        }
        Self { entries }
    }

    pub fn base(&self, name: &str) -> Option<&str> {
        self.entries.get(name).map(String::as_str)
    }

    pub fn record(&mut self, name: &str, fingerprint: &str) {
        self.entries
            .insert(name.to_string(), fingerprint.to_string());
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        use std::io::Write;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut out = String::from(
            "# What this machine and its store last agreed on, per item.\n\
             # Written by kitbag. Fingerprints, not values.\n",
        );
        for (name, print) in &self.entries {
            out.push_str(&format!("{name}\t{print}\n"));
        }
        let mut file = std::fs::File::create(path)?;
        file.write_all(out.as_bytes())?;
        // It names every item this machine exchanges, which is an inventory
        // even without the values.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_machine_that_has_exchanged_nothing_has_an_empty_ledger() {
        assert!(Ledger::load(Path::new("/nonexistent/kitbag/exchanged")).is_empty());
    }

    #[test]
    fn what_is_written_comes_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("exchanged");

        let mut led = Ledger::default();
        led.record("env:a", "aaa");
        led.record("app:b", "bbb");
        led.save(&path).unwrap();

        let back = Ledger::load(&path);
        assert_eq!(back.base("env:a"), Some("aaa"));
        assert_eq!(back.base("app:b"), Some("bbb"));
        assert_eq!(back.base("env:missing"), None);
    }

    #[test]
    fn it_is_owner_only_because_it_names_everything_this_machine_holds() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("exchanged");
        let mut led = Ledger::default();
        led.record("env:a", "aaa");
        led.save(&path).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "mode was {:o}", mode & 0o777);
    }

    #[test]
    fn a_later_record_replaces_an_earlier_one() {
        let mut led = Ledger::default();
        led.record("env:a", "one");
        led.record("env:a", "two");
        assert_eq!(led.base("env:a"), Some("two"));
    }
}
