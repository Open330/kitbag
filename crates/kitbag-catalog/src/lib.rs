//! What is on this machine that nothing is tracking yet.
//!
//! The question this answers is the one a person cannot answer for themselves:
//! *what have I forgotten?* Setting up a new machine is where you find out —
//! usually by hitting the thing that is missing, a week later, in the middle
//! of something else.
//!
//! Two sources. A **catalogue** of places credentials are known to live, which
//! is where the obvious ones come from; and a **heuristic** for the rest, since
//! nobody's catalogue will ever list the file you invented on a Tuesday.
//!
//! Nothing here reads a value out loud. A finding says a path, a guess at whose
//! it is, and why it was noticed.

use std::path::{Path, PathBuf};

/// A place credentials are known to live, and what to assume about it.
pub struct Known {
    /// Relative to the home directory. A trailing `/*` matches one level.
    pub path: &'static str,
    pub scope: &'static str,
    pub why: &'static str,
}

/// The obvious places, in the order a person would think of them.
///
/// A guess at the scope is exactly that: `~/.aws/config` is a work credential
/// far more often than not, and saying so beats leaving every finding blank.
/// The guess is always shown as a guess.
pub const CATALOGUE: &[Known] = &[
    Known {
        path: ".aws/config",
        scope: "work",
        why: "AWS profiles and roles",
    },
    Known {
        path: ".aws/credentials",
        scope: "work",
        why: "AWS access keys",
    },
    Known {
        path: ".kube/config",
        scope: "work",
        why: "cluster credentials",
    },
    Known {
        path: ".kube/*.yaml",
        scope: "work",
        why: "a cluster config",
    },
    Known {
        path: ".docker/config.json",
        scope: "personal",
        why: "registry logins",
    },
    Known {
        path: ".npmrc",
        scope: "personal",
        why: "an npm token",
    },
    Known {
        path: ".pypirc",
        scope: "personal",
        why: "a PyPI token",
    },
    Known {
        path: ".netrc",
        scope: "personal",
        why: "passwords for anything using netrc",
    },
    Known {
        path: ".gitconfig",
        scope: "personal",
        why: "identity, and sometimes a token",
    },
    Known {
        path: ".config/gh/hosts.yml",
        scope: "personal",
        why: "a GitHub token",
    },
    Known {
        path: ".config/rclone/rclone.conf",
        scope: "personal",
        why: "cloud storage credentials",
    },
    Known {
        path: ".terraformrc",
        scope: "work",
        why: "a Terraform Cloud token",
    },
    Known {
        path: ".cargo/credentials.toml",
        scope: "personal",
        why: "a crates.io token",
    },
    Known {
        path: ".claude/.credentials.json",
        scope: "personal",
        why: "an assistant's login",
    },
    Known {
        path: ".codex/auth.json",
        scope: "personal",
        why: "an assistant's login",
    },
    Known {
        path: ".config/hishtory/.hishtory.config.json",
        scope: "personal",
        why: "a shell-history key",
    },
    Known {
        path: ".ssh/id_*",
        scope: "personal",
        why: "a private key",
    },
    Known {
        path: ".envs/*",
        scope: "auto",
        why: "a directory of environment files",
    },
    Known {
        path: "Library/Keychains/*.keychain-db",
        scope: "work",
        why: "a keychain a tool made for itself",
    },
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// A known place.
    Catalogue,
    /// Found by looking: owner-only, and it reads like secrets.
    Noticed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// A file that exists now. For a pattern finding this is the first match,
    /// so there is always something concrete to show and to compare against.
    pub path: PathBuf,
    /// The pattern this came from, when the catalogue named a directory rather
    /// than a file. Proposing `~/.envs/*` instead of each file in it covers
    /// the ones that do not exist yet, which is most of the point.
    pub pattern: Option<String>,
    /// A guess. `auto` means the file should carry its own marker.
    pub scope: String,
    pub why: String,
    pub source: Source,
}

impl Finding {
    /// The lines to paste into a machine file.
    /// What to show: the pattern if there is one, else the file.
    pub fn shown(&self, home: &Path) -> String {
        if let Some(p) = &self.pattern {
            return format!("~/{p}");
        }
        match self.path.strip_prefix(home) {
            Ok(rest) => format!("~/{}", rest.display()),
            Err(_) => self.path.display().to_string(),
        }
    }

    pub fn as_toml(&self, home: &Path) -> String {
        let shown = self.shown(home);
        if self.scope == "auto" {
            format!("[[track]]\npath = \"{shown}\"\n")
        } else {
            format!(
                "[[track]]\npath = \"{shown}\"\nscope = \"{}\"\n",
                self.scope
            )
        }
    }
}

/// Everything worth proposing, minus what is already tracked or dismissed.
pub fn scan(home: &Path, tracked: &[PathBuf], dismissed: &[PathBuf]) -> Vec<Finding> {
    let mut found: Vec<Finding> = Vec::new();

    for known in CATALOGUE {
        let matches = expand(home, known.path);
        if matches.is_empty() {
            continue;
        }
        if known.path.contains('*') {
            // One proposal for the pattern, and only if something there is
            // still untracked - a directory whose files are all accounted for
            // is not news.
            if matches.iter().all(|m| tracked.contains(m)) {
                continue;
            }
            push(
                &mut found,
                Finding {
                    path: matches[0].clone(),
                    pattern: Some(known.path.to_string()),
                    scope: known.scope.to_string(),
                    why: known.why.to_string(),
                    source: Source::Catalogue,
                },
            );
            continue;
        }
        for path in matches {
            push(
                &mut found,
                Finding {
                    path,
                    pattern: None,
                    scope: known.scope.to_string(),
                    why: known.why.to_string(),
                    source: Source::Catalogue,
                },
            );
        }
    }

    for path in noticed(home) {
        push(
            &mut found,
            Finding {
                path,
                pattern: None,
                scope: "auto".into(),
                why: "owner-only, and it reads like credentials".into(),
                source: Source::Noticed,
            },
        );
    }

    found.retain(|f| !tracked.contains(&f.path) && !dismissed.contains(&f.path));
    found.sort_by(|a, b| a.path.cmp(&b.path));
    found
}

fn push(found: &mut Vec<Finding>, finding: Finding) {
    if !found.iter().any(|f| f.path == finding.path) {
        found.push(finding);
    }
}

fn expand(home: &Path, pattern: &str) -> Vec<PathBuf> {
    let full = home.join(pattern);
    if pattern.contains('*') {
        glob::glob(&full.to_string_lossy())
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .filter(|p| p.is_file())
                    .collect()
            })
            .unwrap_or_default()
    } else if full.is_file() {
        vec![full]
    } else {
        Vec::new()
    }
}

/// Files nobody catalogued: owner-only, and holding something that looks like
/// a secret rather than a setting.
///
/// The search is deliberately shallow - the top of the home directory and one
/// level of `~/.config` - because a scan that walks an entire home directory
/// is slow, noisy, and reads a great many files it has no business reading.
fn noticed(home: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut roots = vec![home.to_path_buf()];
    if let Ok(entries) = std::fs::read_dir(home.join(".config")) {
        roots.extend(
            entries
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.is_dir()),
        );
    }

    for root in roots {
        let Ok(entries) = std::fs::read_dir(&root) else {
            continue;
        };
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if !path.is_file() || !owner_only(&path) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue; // binary: the catalogue is the only way in
            };
            if text.len() < 8_000 && looks_like_secrets(&text) {
                out.push(path);
            }
        }
    }
    out
}

fn owner_only(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|m| m.permissions().mode() & 0o077 == 0)
        .unwrap_or(false)
}

/// `key = value`, where some value is long and random-looking. The lint rules
/// already know what a secret looks like; this asks the same question of a
/// whole file.
fn looks_like_secrets(text: &str) -> bool {
    text.lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .filter_map(|l| l.split_once('='))
        .any(|(_, value)| !kitbag_core::lint::check(value).is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    fn write(home: &Path, rel: &str, body: &str, mode: u32) -> PathBuf {
        let path = home.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, body).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
        path
    }

    /// Assembled rather than written down: a literal would be a finding in
    /// this repository's own lint run.
    fn secretish() -> String {
        format!(
            "TOKEN={}{}{}\n",
            "ghp_", "aB3xQ9zK7mP2wR5t", "Y8uI1oL4cV6nE0jH"
        )
    }

    #[test]
    fn the_obvious_places_are_found() {
        let h = tempfile::tempdir().unwrap();
        write(h.path(), ".aws/credentials", "[default]\n", 0o600);
        write(h.path(), ".npmrc", "registry=https://x\n", 0o644);

        let found = scan(h.path(), &[], &[]);
        let paths: Vec<_> = found.iter().map(|f| f.path.clone()).collect();

        assert!(paths.contains(&h.path().join(".aws/credentials")));
        assert!(paths.contains(&h.path().join(".npmrc")));
    }

    #[test]
    fn a_guess_at_the_scope_comes_with_a_reason() {
        let h = tempfile::tempdir().unwrap();
        write(h.path(), ".aws/config", "[profile x]\n", 0o600);

        let found = scan(h.path(), &[], &[]);
        let aws = found.iter().find(|f| f.path.ends_with("config")).unwrap();

        assert_eq!(aws.scope, "work");
        assert!(aws.why.contains("AWS"), "{}", aws.why);
        assert_eq!(aws.source, Source::Catalogue);
    }

    #[test]
    fn a_file_nobody_catalogued_is_still_noticed() {
        let h = tempfile::tempdir().unwrap();
        write(h.path(), ".invented-on-a-tuesday", &secretish(), 0o600);

        let found = scan(h.path(), &[], &[]);
        let mine = found
            .iter()
            .find(|f| f.path.ends_with(".invented-on-a-tuesday"))
            .expect("should have been noticed");

        assert_eq!(mine.source, Source::Noticed);
        assert_eq!(
            mine.scope, "auto",
            "an unknown file must declare its own scope"
        );
    }

    #[test]
    fn a_world_readable_config_is_left_alone() {
        let h = tempfile::tempdir().unwrap();
        write(h.path(), ".ordinary-config", &secretish(), 0o644);

        let found = scan(h.path(), &[], &[]);
        assert!(
            !found.iter().any(|f| f.path.ends_with(".ordinary-config")),
            "owner-only is the filter that keeps this quiet"
        );
    }

    #[test]
    fn a_file_of_plain_settings_is_not_a_finding() {
        let h = tempfile::tempdir().unwrap();
        write(
            h.path(),
            ".just-settings",
            "editor=vim\ncolumns=100\n",
            0o600,
        );

        let found = scan(h.path(), &[], &[]);
        assert!(!found.iter().any(|f| f.path.ends_with(".just-settings")));
    }

    #[test]
    fn a_directory_is_proposed_as_one_pattern_not_a_file_at_a_time() {
        let h = tempfile::tempdir().unwrap();
        write(h.path(), ".envs/a.env", "A=1\n", 0o600);
        write(h.path(), ".envs/b.env", "B=2\n", 0o600);

        let found = scan(h.path(), &[], &[]);
        let envs: Vec<_> = found.iter().filter(|f| f.pattern.is_some()).collect();

        assert_eq!(envs.len(), 1, "one proposal, not one per file");
        assert_eq!(
            envs[0].as_toml(h.path()),
            "[[track]]\npath = \"~/.envs/*\"\n"
        );
    }

    #[test]
    fn a_directory_whose_files_are_all_tracked_is_not_news() {
        let h = tempfile::tempdir().unwrap();
        let a = write(h.path(), ".envs/a.env", "A=1\n", 0o600);

        let found = scan(h.path(), std::slice::from_ref(&a), &[]);
        assert!(!found.iter().any(|f| f.pattern.is_some()));
    }

    #[test]
    fn what_is_already_tracked_is_not_proposed_again() {
        let h = tempfile::tempdir().unwrap();
        let npmrc = write(h.path(), ".npmrc", "registry=x\n", 0o600);

        let found = scan(h.path(), std::slice::from_ref(&npmrc), &[]);
        assert!(!found.iter().any(|f| f.path == npmrc));
    }

    #[test]
    fn a_dismissal_is_remembered() {
        let h = tempfile::tempdir().unwrap();
        let path = write(h.path(), ".pypirc", "[pypi]\n", 0o600);

        let found = scan(h.path(), &[], std::slice::from_ref(&path));
        assert!(
            !found.iter().any(|f| f.path == path),
            "the second run is quiet"
        );
    }

    #[test]
    fn a_finding_prints_the_lines_to_paste() {
        let h = tempfile::tempdir().unwrap();
        write(h.path(), ".aws/config", "[profile x]\n", 0o600);

        let found = scan(h.path(), &[], &[]);
        let toml = found[0].as_toml(h.path());

        assert!(toml.contains("path = \"~/.aws/config\""), "{toml}");
        assert!(toml.contains("scope = \"work\""), "{toml}");
        // and an auto finding says nothing about scope, so the file must
        assert_eq!(
            Finding {
                path: h.path().join(".envs/x.env"),
                pattern: None,
                scope: "auto".into(),
                why: String::new(),
                source: Source::Catalogue,
            }
            .as_toml(h.path()),
            "[[track]]\npath = \"~/.envs/x.env\"\n"
        );
    }
}
