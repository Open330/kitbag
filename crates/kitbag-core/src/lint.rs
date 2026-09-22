//! Refusing to commit the things that must not be committed.
//!
//! A public settings repository is safe only if two kinds of thing stay out of
//! it: the **values** (obviously) and the **inventory** — the list of what
//! exists. `~/.envs/kibana.env → work` names an employer, a stack and a target,
//! and no one needs the secret to make use of that.
//!
//! This module is the rule set. `kitbag lint` runs it over a working tree as a
//! pre-commit hook and in CI; kitbag's own repository runs it over itself,
//! because a tool that leaks its author's machine while being written has
//! argued against its own design.
//!
//! Findings never quote what they found. A linter that prints the secret it
//! caught has moved it into a build log.

use std::collections::HashMap;

/// One thing that should not be in a file that other people can read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub rule: &'static str,
    pub line: usize,
    /// What it looked like, with the body removed: `ghp_****` and nothing more.
    pub masked: String,
    pub why: &'static str,
}

/// Prefixes that are only ever the start of a credential.
const PREFIXES: &[(&str, &str)] = &[
    ("ghp_", "GitHub personal access token"),
    ("gho_", "GitHub OAuth token"),
    ("ghs_", "GitHub server token"),
    ("github_pat_", "GitHub fine-grained token"),
    ("glpat-", "GitLab personal access token"),
    ("xoxb-", "Slack bot token"),
    ("xoxp-", "Slack user token"),
    ("sk-", "OpenAI-style API key"),
    ("AKIA", "AWS access key id"),
    ("ASIA", "AWS temporary access key id"),
    ("AIza", "Google API key"),
    ("AGE-SECRET-KEY-", "age private key"),
    ("hf_", "Hugging Face token"),
];

/// The rule set has to name what it looks for, which makes this file look like
/// the thing it is guarding against. `lint:allow` is how a line says so.
const BLOCKS: &[(&str, &str)] = &[
    ("BEGIN OPENSSH PRIVATE KEY", "private key"), // lint:allow
    ("BEGIN RSA PRIVATE KEY", "private key"),     // lint:allow
    ("BEGIN PGP PRIVATE KEY BLOCK", "private key"), // lint:allow
    ("BEGIN EC PRIVATE KEY", "private key"),      // lint:allow
];

/// Absolute paths that only exist on one person's machine. They leak a
/// username, sometimes an employer, and they break for everyone else.
///
/// The prefix that matched comes back with the reason, so the report can show
/// the one actually found. Naming the wrong prefix sends whoever reads it
/// looking on a machine that may not even have that directory — and this
/// comment cannot spell the other one out, because this file is checked by
/// the rule it describes. // lint:allow
fn personal_path(line: &str) -> Option<(&'static str, &'static str)> {
    for prefix in ["/Users/", "/home/"] {
        if let Some(idx) = line.find(prefix) {
            let rest = &line[idx + prefix.len()..];
            let user: String = rest.chars().take_while(|c| c.is_alphanumeric()).collect();
            // `/home/runner` is CI; `/Users/` with nothing after is a doc example.
            if !user.is_empty() && user != "runner" && user != "user" && user != "you" {
                return Some((prefix, "an absolute path from somebody's machine"));
            }
        }
    }
    None
}

/// Shannon entropy per character. Real prose sits near 4; a random 32-character
/// secret sits above 4.5 and rarely below.
fn entropy(s: &str) -> f64 {
    let mut counts: HashMap<char, usize> = HashMap::new();
    for c in s.chars() {
        *counts.entry(c).or_default() += 1;
    }
    let len = s.chars().count() as f64;
    counts
        .values()
        .map(|&n| {
            let p = n as f64 / len;
            -p * p.log2()
        })
        .sum()
}

fn looks_like_a_secret(word: &str) -> bool {
    let len = word.len();
    if !(32..=200).contains(&len) {
        return false;
    }
    if !word
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "+/=_-.".contains(c))
    {
        return false;
    }
    // A hash written down on purpose (a lockfile, a pinned checksum) is all
    // hex and therefore low-entropy for its length. Those are fine.
    let hexish = word.chars().all(|c| c.is_ascii_hexdigit());
    !hexish && entropy(word) > 4.2
}

fn mask(word: &str) -> String {
    let head: String = word.chars().take(4).collect();
    format!("{head}**** ({} chars)", word.len())
}

/// Everything wrong with one file.
pub fn check(content: &str) -> Vec<Finding> {
    let mut findings = Vec::new();

    for (n, line) in content.lines().enumerate() {
        let line_no = n + 1;

        // A line that says "this is what a token looks like" is documentation,
        // not a leak. Without this the rule set cannot be written down.
        let documented = line.contains("lint:allow");

        for (prefix, what) in PREFIXES {
            if let Some(idx) = line.find(prefix) {
                let word: String = line[idx..]
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || "_-".contains(*c))
                    .collect();
                if word.len() > prefix.len() + 8 && !documented {
                    findings.push(Finding {
                        rule: "credential-prefix",
                        line: line_no,
                        masked: mask(&word),
                        why: what,
                    });
                }
            }
        }

        for (marker, what) in BLOCKS {
            if line.contains(marker) && !documented {
                findings.push(Finding {
                    rule: "private-key",
                    line: line_no,
                    masked: "-----BEGIN ****".into(),
                    why: what,
                });
            }
        }

        if let Some((prefix, why)) = personal_path(line) {
            if !documented {
                findings.push(Finding {
                    rule: "personal-path",
                    line: line_no,
                    masked: format!("{prefix}****"),
                    why,
                });
            }
        }

        // Only if nothing more specific already named this line. A token caught
        // by its prefix reported twice is a rule set that cries wolf.
        let already = findings.iter().any(|f| f.line == line_no);
        if !documented && !already {
            for word in line.split(|c: char| c.is_whitespace() || "\"'`=,;()[]{}".contains(c)) {
                if looks_like_a_secret(word) {
                    findings.push(Finding {
                        rule: "high-entropy",
                        line: line_no,
                        masked: mask(word),
                        why: "a long random-looking string",
                    });
                    break;
                }
            }
        }
    }

    findings
}

#[cfg(test)]
mod tests {
    use super::*;

    // Built at runtime, never written down: a literal here would be a finding
    // in this very file, and would trip every scanner between here and GitHub.
    fn fake(prefix: &str, len: usize) -> String {
        let body: String = "aB3xQ9zK7mP2wR5tY8uI1oL4"
            .chars()
            .cycle()
            .take(len)
            .collect();
        format!("{prefix}{body}")
    }

    #[test]
    fn catches_a_token_by_its_prefix() {
        let line = format!("export TOKEN={}", fake("ghp_", 36));
        let f = check(&line);
        assert_eq!(f.len(), 1, "{f:?}");
        assert_eq!(f[0].rule, "credential-prefix");
    }

    #[test]
    fn one_line_is_reported_once() {
        // prefix and entropy both match; the specific rule wins
        let f = check(&format!("export TOKEN={}", fake("ghp_", 60)));
        assert_eq!(f.len(), 1, "{f:?}");
        assert_eq!(f[0].rule, "credential-prefix");
    }

    #[test]
    fn never_prints_what_it_caught() {
        let secret = fake("ghp_", 36);
        let f = check(&format!("token = {secret}"));
        for finding in &f {
            assert!(!secret.contains(&finding.masked));
            assert!(finding.masked.contains("****"));
        }
    }

    #[test]
    fn catches_a_private_key_header() {
        let f = check("-----BEGIN OPENSSH PRIVATE KEY-----"); // lint:allow
        assert_eq!(f[0].rule, "private-key");
    }

    #[test]
    fn catches_a_path_from_somebody_machine() {
        let f = check("source /Users/alice/.envs/work.env"); // lint:allow
        assert_eq!(f[0].rule, "personal-path");
        assert_eq!(f[0].masked, "/Users/****");
        // and says which prefix it found, rather than always the first one
        let linux = check("source /home/alice/.envs/work.env"); // lint:allow
        assert_eq!(linux[0].masked, "/home/****");
        // and does not object to the paths CI and documentation really use
        assert!(check("/home/runner/work/kitbag").is_empty());
        assert!(check("~/.config/kitbag/machine.toml").is_empty());
    }

    #[test]
    fn leaves_prose_and_hashes_alone() {
        assert!(check("The quick brown fox jumps over the lazy dog, twice.").is_empty());
        assert!(check(
            "sha256 = \"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855\""
        )
        .is_empty());
        assert!(check("path = \"~/.envs/*.env\"").is_empty());
    }

    #[test]
    fn a_documented_example_is_allowed_to_say_what_it_is() {
        let line = format!("{} # lint:allow — shape of a token", fake("ghp_", 36));
        assert!(check(&line).is_empty());
    }

    #[test]
    fn catches_a_bare_high_entropy_string() {
        let f = check(&format!("password: {}", fake("", 44)));
        assert_eq!(f[0].rule, "high-entropy");
    }
}
