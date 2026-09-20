//! A file says what it is.
//!
//! The scope of a secret cannot live in a public repository - a table mapping
//! `kibana.env` to an employer is the leak. It lives in the file instead, as a
//! comment in the first few lines, so it travels with the file when the file is
//! copied to another machine.

use std::str::FromStr;

use crate::scope::{Scope, UnknownScope};

/// How many lines of a file are searched for its markers.
const HEAD: usize = 5;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Markers {
    pub scope: Option<Scope>,
    pub owner: Option<String>,
    pub spans: Vec<String>,
}

/// Reads `# scope:`, `# owner:` and `# spans:` out of the head of a file.
///
/// Anything that takes `#` comments works: env files, ssh configs,
/// authorized_keys, npmrc, gitconfig. A file that cannot hold a comment - a
/// private key, a keychain - is declared in the machine profile instead.
pub fn parse(content: &str) -> Result<Markers, UnknownScope> {
    let mut m = Markers::default();
    for line in content.lines().take(HEAD) {
        let Some(rest) = line.trim_start().strip_prefix('#') else {
            continue;
        };
        let rest = rest.trim_start();
        if let Some(v) = strip_key(rest, "scope:") {
            m.scope = Some(Scope::from_str(v)?);
        } else if let Some(v) = strip_key(rest, "owner:") {
            m.owner = Some(v.trim().to_string());
        } else if let Some(v) = strip_key(rest, "spans:") {
            m.spans = v
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
        }
    }
    if let (Some(Scope::Mixed { .. }), false) = (&m.scope, m.spans.is_empty()) {
        m.scope = Some(Scope::Mixed {
            spans: m.spans.clone(),
        });
    }
    Ok(m)
}

fn strip_key<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let lower = line.to_ascii_lowercase();
    lower.starts_with(key).then(|| line[key.len()..].trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_scope_and_owner() {
        let m = parse("# scope: work\n# owner: acme\nexport TOKEN=x\n").unwrap();
        assert_eq!(m.scope, Some(Scope::Work));
        assert_eq!(m.owner.as_deref(), Some("acme"));
    }

    #[test]
    fn mixed_carries_what_it_spans() {
        let m = parse("# scope: mixed\n# spans: personal, work\nHost x\n").unwrap();
        assert_eq!(
            m.scope,
            Some(Scope::Mixed {
                spans: vec!["personal".into(), "work".into()]
            })
        );
    }

    #[test]
    fn a_marker_below_the_head_is_not_a_marker() {
        let body = "\n\n\n\n\n# scope: work\n";
        assert_eq!(parse(body).unwrap().scope, None);
    }

    #[test]
    fn an_unmarked_file_says_nothing_rather_than_guessing() {
        assert_eq!(parse("export TOKEN=x\n").unwrap().scope, None);
    }

    #[test]
    fn a_typo_is_refused() {
        assert!(parse("# scope: wrok\n").is_err());
    }
}
