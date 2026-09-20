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
            payload,
            extra: BTreeMap::new(),
        }
    }

    pub fn with_owner(mut self, owner: Option<String>) -> Self {
        self.owner = owner;
        self
    }

    pub fn sha256(&self) -> String {
        let mut h = Sha256::new();
        h.update(&self.payload);
        format!("{:x}", h.finalize())
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
