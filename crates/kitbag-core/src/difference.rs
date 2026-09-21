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
