//! What differs between what a machine holds and what a store holds — without
//! saying what either of them is.
//!
//! A report that an item "changed" is not enough to act on. Moving this
//! repository's machines across meant, for every such item, reading both sides
//! by hand to find that one held four keys and the other two — and then
//! deciding which way it should go. That reading is mechanical, and the part
//! that is not mechanical is the deciding, so this does the first and leaves
//! the second.
//!
//! Nothing here returns a value. Key names, counts, sizes and hashes only, and
//! a name is only a name if it looks like one: the same check that stopped a
//! status report printing the body of a private key as though it were a list
//! of settings.

use std::collections::BTreeMap;

use crate::envelope::payload_hash;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Difference {
    /// Both sides parse as `key = value` lines.
    Keys {
        only_here: Vec<String>,
        only_there: Vec<String>,
        /// Present on both sides, holding something different.
        differing: Vec<String>,
        here_lines: usize,
        there_lines: usize,
    },
    /// One or both sides are not settings: a key, a tarball, an app's export.
    /// Size and hash is all that can be said without saying the contents.
    Opaque {
        here_bytes: usize,
        there_bytes: usize,
        here_hash: String,
        there_hash: String,
    },
    /// Both sides are archives, so what is in them can be said without
    /// saying what any of it contains.
    Archive {
        only_here: Vec<String>,
        only_there: Vec<String>,
        /// Present on both sides at a different size.
        differing: Vec<String>,
        here_count: usize,
        there_count: usize,
    },
    /// The bytes are the same. Nothing to resolve.
    None,
}

impl Difference {
    pub fn is_none(&self) -> bool {
        matches!(self, Difference::None)
    }
}

/// `key -> hash of its value`. The hash is how two values are compared without
/// either being held or shown.
fn pairs(text: &str) -> Option<BTreeMap<String, String>> {
    // A key file is not a settings file, whatever its punctuation suggests.
    if text.contains("PRIVATE KEY") || text.starts_with("ssh-") {
        return None;
    }

    let mut out = BTreeMap::new();
    let mut lines = 0usize;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        lines += 1;
        let Some((left, right)) = line.split_once('=') else {
            continue;
        };
        let key = left.trim().trim_start_matches("export ").trim();
        if !crate::collect::is_a_name(key) {
            continue;
        }
        out.insert(key.to_string(), payload_hash(right.trim().as_bytes()));
    }

    // Lines that are not `key = value` mean this is something else being read
    // as settings; half a picture is worse than admitting to none.
    if out.is_empty() || out.len() * 2 < lines {
        return None;
    }
    Some(out)
}

/// What an archive holds: path to size. `None` when it is not one, or not one
/// this can read — a guess about a backup is worse than no guess.
fn entries(bytes: &[u8]) -> Option<BTreeMap<String, u64>> {
    if bytes.len() < 2 || bytes[0] != 0x1f || bytes[1] != 0x8b {
        return None;
    }
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(bytes));
    let mut out = BTreeMap::new();
    for entry in archive.entries().ok()? {
        let entry = entry.ok()?;
        let path = entry.path().ok()?.display().to_string();
        // A path is structure, not a value — but a path that looks like a
        // credential is still not printed.
        if !crate::lint::check(&path).is_empty() {
            continue;
        }
        out.insert(path, entry.size());
    }
    (!out.is_empty()).then_some(out)
}

fn lines_of(text: &str) -> usize {
    text.lines().filter(|l| !l.trim().is_empty()).count()
}

/// What differs, in terms of whatever the two sides turn out to be.
pub fn describe(here: &[u8], there: &[u8]) -> Difference {
    if here == there {
        return Difference::None;
    }

    let both = std::str::from_utf8(here)
        .ok()
        .zip(std::str::from_utf8(there).ok());
    if let Some((here_text, there_text)) = both {
        if let (Some(a), Some(b)) = (pairs(here_text), pairs(there_text)) {
            let only_here: Vec<String> =
                a.keys().filter(|k| !b.contains_key(*k)).cloned().collect();
            let only_there: Vec<String> =
                b.keys().filter(|k| !a.contains_key(*k)).cloned().collect();
            let differing: Vec<String> = a
                .iter()
                .filter(|(k, v)| b.get(*k).is_some_and(|other| other != *v))
                .map(|(k, _)| k.clone())
                .collect();
            return Difference::Keys {
                only_here,
                only_there,
                differing,
                here_lines: lines_of(here_text),
                there_lines: lines_of(there_text),
            };
        }
    }

    if let (Some(a), Some(b)) = (entries(here), entries(there)) {
        let only_here: Vec<String> = a.keys().filter(|k| !b.contains_key(*k)).cloned().collect();
        let only_there: Vec<String> = b.keys().filter(|k| !a.contains_key(*k)).cloned().collect();
        let differing: Vec<String> = a
            .iter()
            .filter(|(k, size)| b.get(*k).is_some_and(|other| other != *size))
            .map(|(k, _)| k.clone())
            .collect();
        return Difference::Archive {
            only_here,
            only_there,
            differing,
            here_count: a.len(),
            there_count: b.len(),
        };
    }

    Difference::Opaque {
        here_bytes: here.len(),
        there_bytes: there.len(),
        here_hash: payload_hash(here),
        there_hash: payload_hash(there),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_case_that_made_this_necessary() {
        // One machine held two of these and the store held four. Reading that
        // off a status report was not possible; it took reading both files.
        let here = b"# scope: personal\nDOCS_HOST=a\nDOCS_URL=b\n";
        let there = b"# scope: personal\nDOCS_HOST=a\nDOCS_USER=u\nDOCS_ROOT=r\nDOCS_URL=b\n";
        match describe(here, there) {
            Difference::Keys {
                only_here,
                only_there,
                differing,
                ..
            } => {
                assert!(only_here.is_empty());
                assert_eq!(only_there, vec!["DOCS_ROOT", "DOCS_USER"]);
                assert!(differing.is_empty(), "the shared ones hold the same thing");
            }
            other => panic!("expected keys, got {other:?}"),
        }
    }

    #[test]
    fn a_value_that_changed_is_named_and_not_shown() {
        let here = b"A=one\nB=same\n";
        let there = b"A=two\nB=same\n";
        match describe(here, there) {
            Difference::Keys { differing, .. } => assert_eq!(differing, vec!["A"]),
            other => panic!("expected keys, got {other:?}"),
        }
        // The whole point: neither value appears anywhere in the answer.
        let rendered = format!("{:?}", describe(here, there));
        assert!(
            !rendered.contains("one") && !rendered.contains("two"),
            "{rendered}"
        );
    }

    #[test]
    fn a_private_key_is_never_read_as_settings() {
        // Assembled, so this file does not carry a key-shaped literal.
        let key = format!(
            "-----BEGIN OPENSSH {}-----\nb3BlbnNza\nAAAA=x\n-----END OPENSSH {}-----\n",
            "PRIVATE KEY", "PRIVATE KEY"
        );
        let other = key.replace("b3BlbnNza", "c3BlbnNza");
        match describe(key.as_bytes(), other.as_bytes()) {
            Difference::Opaque { here_bytes, .. } => assert_eq!(here_bytes, key.len()),
            other => panic!("a key must never be described by its 'keys': {other:?}"),
        }
    }

    #[test]
    fn a_tarball_is_a_size_and_a_hash() {
        let here = vec![0x1f, 0x8b, 0x08, 0x00, 1, 2, 3];
        let there = vec![0x1f, 0x8b, 0x08, 0x00, 9, 9, 9, 9];
        match describe(&here, &there) {
            Difference::Opaque {
                here_bytes,
                there_bytes,
                here_hash,
                there_hash,
            } => {
                assert_eq!((here_bytes, there_bytes), (7, 8));
                assert_ne!(here_hash, there_hash);
            }
            other => panic!("expected opaque, got {other:?}"),
        }
    }

    #[test]
    fn the_same_bytes_have_nothing_to_resolve() {
        assert!(describe(b"A=1\n", b"A=1\n").is_none());
    }
}

#[cfg(test)]
mod archive_tests {
    use super::*;
    use std::io::Write;

    fn targz(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut tar = tar::Builder::new(Vec::new());
        for (name, body) in files {
            let mut header = tar::Header::new_gnu();
            header.set_size(body.len() as u64);
            header.set_mode(0o600);
            header.set_cksum();
            tar.append_data(&mut header, name, *body).unwrap();
        }
        let raw = tar.into_inner().unwrap();
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        gz.write_all(&raw).unwrap();
        gz.finish().unwrap()
    }

    #[test]
    fn an_archive_is_described_by_what_is_in_it() {
        // Two byte counts is not something a person can decide on, which is
        // what a real resolve came down to for a widget directory.
        let here = targz(&[("app/prefs.json", b"{}"), ("app/only-here.json", b"{}")]);
        let there = targz(&[
            ("app/prefs.json", b"{\"a\":1}"),
            ("app/only-there.json", b"{}"),
        ]);

        match describe(&here, &there) {
            Difference::Archive {
                only_here,
                only_there,
                differing,
                here_count,
                there_count,
            } => {
                assert_eq!((here_count, there_count), (2, 2));
                assert_eq!(only_here, vec!["app/only-here.json"]);
                assert_eq!(only_there, vec!["app/only-there.json"]);
                assert_eq!(differing, vec!["app/prefs.json"], "same path, other size");
            }
            other => panic!("expected an archive, got {other:?}"),
        }
    }

    #[test]
    fn what_is_not_an_archive_is_still_a_size_and_a_hash() {
        assert!(matches!(
            describe(b"not gzip", b"nor this"),
            Difference::Opaque { .. }
        ));
    }

    #[test]
    fn a_path_that_looks_like_a_credential_is_left_out() {
        // Paths are structure, not values — but one that trips the lint is
        // not printed just because it happens to be a filename.
        // Assembled, so this file does not itself carry a credential-shaped
        // literal — kitbag lints its own source, and it is right to.
        let secret = format!(
            "app/{}_{}{}",
            "ghp", "0123456789abcdefghij", "klmnopqrstuvwx"
        );
        let here = targz(&[("app/fine.json", b"{}"), (secret.as_str(), b"{}")]);
        let there = targz(&[("app/fine.json", b"{\"a\":1}")]);
        match describe(&here, &there) {
            Difference::Archive { only_here, .. } => {
                assert!(
                    only_here.is_empty(),
                    "a credential-shaped path leaked: {only_here:?}"
                );
            }
            other => panic!("expected an archive, got {other:?}"),
        }
    }
}
