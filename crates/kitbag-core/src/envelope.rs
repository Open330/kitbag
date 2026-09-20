//! What kitbag actually stores, whatever store it is stored in.
//!
//! Secret stores disagree about everything: Bitwarden has notes, custom fields
//! and attachments; 1Password has typed fields and files; `pass` has a tree of
//! GPG-encrypted text files with no metadata at all; `age` has a file. The one
//! thing all of them can do is keep **bytes under a name**.
//!
//! So kitbag keeps its own envelope and asks a backend only to store it. A
//! backend that has native fields may mirror the header into them, so the web
//! UI shows the scope — but the envelope stays the source of truth, and adding
//! a backend never means teaching it about scopes.
//!
//! ```text
//! kitbag/1
//! scope: work
//! owner: acme
//! encoding: utf8
//! sha256: 1f0e3d…
//!
//! export TOKEN=…
//! ```
//!
//! The payload is base64 when it is not valid UTF-8, because a store that only
//! holds text is the common case, not the exception.

use std::collections::BTreeMap;
use std::str::FromStr;

use base64::Engine as _;
use sha2::{Digest, Sha256};

use crate::scope::{Scope, UnknownScope};

const MAGIC: &str = "kitbag/1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Envelope {
    pub scope: Scope,
    pub owner: Option<String>,
    /// Where this belongs, written `~/…`. A restore onto a machine where the
    /// file does not exist yet - the whole point of a restore - has no other
    /// way to know, since there is nothing on disk to learn it from.
    pub path: Option<String>,
    pub payload: Vec<u8>,
    /// Headers kitbag did not recognise, kept so a newer writer does not lose
    /// information when an older reader rewrites the item.
    pub extra: BTreeMap<String, String>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum EnvelopeError {
    #[error("not a kitbag envelope")]
    NotAnEnvelope,
    #[error("envelope has no scope")]
    MissingScope,
    #[error(transparent)]
    Scope(#[from] UnknownScope),
    #[error("payload is not valid base64")]
    BadBase64,
    #[error(
        "payload does not match its sha256 — the store returned something else than was written"
    )]
    Corrupt,
}

impl Envelope {
    pub fn new(scope: Scope, payload: Vec<u8>) -> Self {
        Self {
            scope,
            owner: None,
            path: None,
            payload,
            extra: BTreeMap::new(),
        }
    }

    pub fn with_owner(mut self, owner: Option<String>) -> Self {
        self.owner = owner;
        self
    }

    pub fn with_path(mut self, path: Option<String>) -> Self {
        self.path = path;
        self
    }

    /// Where this may be written, given a home directory.
    ///
    /// A store decides where a restore puts things, so a store that has been
    /// tampered with must not be able to name `../../etc/anything`. Only paths
    /// under the home directory are allowed, and `..` is refused outright.
    pub fn destination(&self, home: &std::path::Path) -> Option<std::path::PathBuf> {
        let raw = self.path.as_deref()?;
        if raw.contains("..") {
            return None;
        }
        let rest = raw.strip_prefix("~/")?;
        let full = home.join(rest);
        full.starts_with(home).then_some(full)
    }

    pub fn sha256(&self) -> String {
        payload_hash(&self.payload)
    }

    pub fn to_text(&self) -> String {
        let utf8 = std::str::from_utf8(&self.payload).ok();
        let (encoding, body) = match utf8 {
            Some(s) => ("utf8", s.to_string()),
            None => (
                "base64",
                base64::engine::general_purpose::STANDARD.encode(&self.payload),
            ),
        };

        let mut out = String::from(MAGIC);
        out.push('\n');
        out.push_str(&format!("scope: {}\n", self.scope));
        if let Scope::Mixed { spans } = &self.scope {
            if !spans.is_empty() {
                out.push_str(&format!("spans: {}\n", spans.join(", ")));
            }
        }
        if let Some(owner) = &self.owner {
            out.push_str(&format!("owner: {owner}\n"));
        }
        if let Some(path) = &self.path {
            out.push_str(&format!("path: {path}\n"));
        }
        out.push_str(&format!("encoding: {encoding}\n"));
        out.push_str(&format!("sha256: {}\n", self.sha256()));
        for (k, v) in &self.extra {
            out.push_str(&format!("{k}: {v}\n"));
        }
        out.push('\n');
        out.push_str(&body);
        out
    }

    pub fn parse(text: &str) -> Result<Self, EnvelopeError> {
        let rest = text
            .strip_prefix(MAGIC)
            .and_then(|r| r.strip_prefix('\n'))
            .ok_or(EnvelopeError::NotAnEnvelope)?;

        let (head, body) = rest.split_once("\n\n").unwrap_or((rest, ""));

        let mut scope: Option<Scope> = None;
        let mut spans: Vec<String> = Vec::new();
        let mut owner = None;
        let mut path = None;
        let mut encoding = "utf8".to_string();
        let mut sha = None;
        let mut extra = BTreeMap::new();

        for line in head.lines() {
            let Some((k, v)) = line.split_once(':') else {
                continue;
            };
            let (k, v) = (k.trim().to_ascii_lowercase(), v.trim().to_string());
            match k.as_str() {
                "scope" => scope = Some(Scope::from_str(&v)?),
                "spans" => {
                    spans = v
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect()
                }
                "owner" => owner = Some(v),
                "path" => path = Some(v),
                "encoding" => encoding = v,
                "sha256" => sha = Some(v),
                _ => {
                    extra.insert(k, v);
                }
            }
        }

        let mut scope = scope.ok_or(EnvelopeError::MissingScope)?;
        if let Scope::Mixed { .. } = scope {
            scope = Scope::Mixed { spans };
        }

        let payload = match encoding.as_str() {
            "base64" => base64::engine::general_purpose::STANDARD
                .decode(body.trim())
                .map_err(|_| EnvelopeError::BadBase64)?,
            _ => body.as_bytes().to_vec(),
        };

        let envelope = Envelope {
            scope,
            owner,
            path,
            payload,
            extra,
        };

        // A store that hands back something other than what was written is a
        // bug worth finding now, not after it has been restored over a file.
        if let Some(sha) = sha {
            if sha != envelope.sha256() {
                return Err(EnvelopeError::Corrupt);
            }
        }
        Ok(envelope)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_survives_a_round_trip() {
        let e = Envelope::new(Scope::Work, b"export TOKEN=abc\n".to_vec())
            .with_owner(Some("acme".into()));
        let back = Envelope::parse(&e.to_text()).unwrap();
        assert_eq!(back, e);
        assert!(e.to_text().contains("export TOKEN=abc"));
    }

    #[test]
    fn binary_survives_a_store_that_only_holds_text() {
        let bytes = vec![0u8, 159, 146, 150, 255, 0, 1];
        let e = Envelope::new(Scope::Personal, bytes.clone());
        let text = e.to_text();
        assert!(text.contains("encoding: base64"));
        assert_eq!(Envelope::parse(&text).unwrap().payload, bytes);
    }

    #[test]
    fn mixed_keeps_what_it_spans() {
        let e = Envelope::new(
            Scope::Mixed {
                spans: vec!["personal".into(), "work".into()],
            },
            b"blob".to_vec(),
        );
        assert_eq!(Envelope::parse(&e.to_text()).unwrap().scope, e.scope);
    }

    #[test]
    fn a_header_this_version_does_not_know_is_kept() {
        let mut e = Envelope::new(Scope::Personal, b"x".to_vec());
        e.extra.insert("rotated-at".into(), "2026-09-20".into());
        let back = Envelope::parse(&e.to_text()).unwrap();
        assert_eq!(
            back.extra.get("rotated-at").map(String::as_str),
            Some("2026-09-20")
        );
    }

    #[test]
    fn an_envelope_carries_where_it_belongs() {
        let e =
            Envelope::new(Scope::Personal, b"x".to_vec()).with_path(Some("~/.envs/a.env".into()));
        let back = Envelope::parse(&e.to_text()).unwrap();
        assert_eq!(back.path.as_deref(), Some("~/.envs/a.env"));
        assert_eq!(
            back.destination(std::path::Path::new("/home/user")),
            Some(std::path::PathBuf::from("/home/user/.envs/a.env"))
        );
    }

    #[test]
    fn a_store_cannot_name_a_path_outside_the_home() {
        let home = std::path::Path::new("/home/user");
        for hostile in ["/etc/passwd", "~/../../etc/passwd", "../elsewhere"] {
            let e = Envelope::new(Scope::Personal, b"x".to_vec()).with_path(Some(hostile.into()));
            assert_eq!(e.destination(home), None, "{hostile} should be refused");
        }
    }

    #[test]
    fn a_payload_that_changed_underneath_is_refused() {
        let e = Envelope::new(Scope::Personal, b"correct".to_vec());
        let tampered = e.to_text().replace("correct", "wrong!!");
        assert_eq!(Envelope::parse(&tampered), Err(EnvelopeError::Corrupt));
    }

    #[test]
    fn something_that_is_not_an_envelope_says_so() {
        assert_eq!(
            Envelope::parse("just a note someone wrote"),
            Err(EnvelopeError::NotAnEnvelope)
        );
    }

    #[test]
    fn an_envelope_without_a_scope_is_refused() {
        let text = format!("{MAGIC}\nowner: acme\n\nbody");
        assert_eq!(Envelope::parse(&text), Err(EnvelopeError::MissingScope));
    }
}

/// What [`Envelope::sha256`] reports, for bytes that are not in an envelope
/// yet: the hash covers the payload and nothing else, so a file on disk can be
/// compared against what a store says it holds without fetching it.
pub fn payload_hash(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    format!("{:x}", h.finalize())
}
