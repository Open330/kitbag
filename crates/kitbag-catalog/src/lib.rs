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
    /// This belongs to the machine, not to its owner: every machine has one
    /// at the same path and they are all different. Proposed with
    /// `per_machine = true`, which names the item after the machine and
    /// leaves the path alone.
    pub per_machine: bool,
    /// What this is, for a report that groups rather than lists forty things.
    pub kind: Kind,
    /// Which of the matches are worth keeping. See `Track::only`.
    pub only: Option<&'static str>,
}

/// The two reasons a file is worth keeping, which are not the same reason.
///
/// A credential is worth keeping because losing it costs you an account. A
/// shell profile is worth keeping because rebuilding it costs you an evening
/// and you will not remember the half of it. Both belong in the store; telling
/// somebody which is which is the difference between a list they read and a
/// list they scroll past.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Values that must not leak.
    Secret,
    /// Configuration and scripts. Yours, and nobody else's business, but
    /// losing them costs time rather than access.
    Setup,
}

impl Kind {
    pub fn heading(self) -> &'static str {
        match self {
            Kind::Secret => "credentials and keys",
            Kind::Setup => "setup — configuration and scripts",
        }
    }
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
        per_machine: false,
        kind: Kind::Secret,
        only: None,
    },
    Known {
        path: ".aws/credentials",
        scope: "work",
        why: "AWS access keys",
        per_machine: false,
        kind: Kind::Secret,
        only: None,
    },
    Known {
        path: ".kube/config",
        scope: "work",
        why: "cluster credentials",
        per_machine: false,
        kind: Kind::Secret,
        only: None,
    },
    Known {
        path: ".kube/*.yaml",
        scope: "work",
        why: "a cluster config",
        per_machine: false,
        kind: Kind::Secret,
        only: None,
    },
    Known {
        path: ".docker/config.json",
        scope: "personal",
        why: "registry logins",
        per_machine: false,
        kind: Kind::Secret,
        only: None,
    },
    Known {
        path: ".npmrc",
        scope: "personal",
        why: "an npm token",
        per_machine: false,
        kind: Kind::Secret,
        only: None,
    },
    Known {
        path: ".pypirc",
        scope: "personal",
        why: "a PyPI token",
        per_machine: false,
        kind: Kind::Secret,
        only: None,
    },
    Known {
        path: ".netrc",
        scope: "personal",
        why: "passwords for anything using netrc",
        per_machine: false,
        kind: Kind::Secret,
        only: None,
    },
    Known {
        path: ".gitconfig",
        scope: "personal",
        why: "identity, and sometimes a token",
        per_machine: false,
        kind: Kind::Secret,
        only: None,
    },
    Known {
        path: ".config/gh/hosts.yml",
        scope: "personal",
        why: "a GitHub token",
        per_machine: false,
        kind: Kind::Secret,
        only: None,
    },
    Known {
        path: ".config/rclone/rclone.conf",
        scope: "personal",
        why: "cloud storage credentials",
        per_machine: false,
        kind: Kind::Secret,
        only: None,
    },
    Known {
        path: ".terraformrc",
        scope: "work",
        why: "a Terraform Cloud token",
        per_machine: false,
        kind: Kind::Secret,
        only: None,
    },
    Known {
        path: ".cargo/credentials.toml",
        scope: "personal",
        why: "a crates.io token",
        per_machine: false,
        kind: Kind::Secret,
        only: None,
    },
    Known {
        path: ".claude/.credentials.json",
        scope: "personal",
        why: "an assistant's login",
        per_machine: false,
        kind: Kind::Secret,
        only: None,
    },
    Known {
        path: ".codex/auth.json",
        scope: "personal",
        why: "an assistant's login",
        per_machine: false,
        kind: Kind::Secret,
        only: None,
    },
    Known {
        path: ".config/hishtory/.hishtory.config.json",
        scope: "personal",
        why: "a shell-history key",
        per_machine: false,
        kind: Kind::Secret,
        only: None,
    },
    Known {
        // Every machine keeps one here and they are all different keys. One
        // item name for four of them means three are backed up nowhere, and a
        // key that exists in one place is gone with the machine it is on.
        // This was learnt the expensive way on four machines; a new one
        // should not have to learn it again.
        path: ".ssh/id_*",
        scope: "personal",
        why: "a private key, and this machine's own",
        per_machine: true,
        kind: Kind::Secret,
        only: None,
    },
    Known {
        path: ".envs/*",
        scope: "auto",
        why: "a directory of environment files",
        per_machine: false,
        kind: Kind::Secret,
        only: None,
    },
    Known {
        path: "Library/Keychains/*.keychain-db",
        scope: "work",
        why: "a keychain a tool made for itself",
        per_machine: false,
        kind: Kind::Secret,
        only: None,
    },
    // Everything below here is the other half: not a credential, and the part
    // a settings repository usually carries in git. A machine that came back
    // with every secret intact and none of this is a machine somebody still
    // has to spend an evening on.
    Known {
        path: ".zshrc",
        scope: "personal",
        why: "your shell, as you set it up",
        per_machine: false,
        kind: Kind::Setup,
        only: None,
    },
    Known {
        path: ".zshenv",
        scope: "personal",
        why: "shell environment",
        per_machine: false,
        kind: Kind::Setup,
        only: None,
    },
    Known {
        path: ".zprofile",
        scope: "personal",
        why: "shell login setup",
        per_machine: false,
        kind: Kind::Setup,
        only: None,
    },
    Known {
        path: ".bashrc",
        scope: "personal",
        why: "your shell, as you set it up",
        per_machine: false,
        kind: Kind::Setup,
        only: None,
    },
    Known {
        path: ".profile",
        scope: "personal",
        why: "shell login setup",
        per_machine: false,
        kind: Kind::Setup,
        only: None,
    },
    Known {
        path: ".gitignore_global",
        scope: "personal",
        why: "what git ignores everywhere",
        per_machine: false,
        kind: Kind::Setup,
        only: None,
    },
    Known {
        path: ".tmux.conf",
        scope: "personal",
        why: "tmux, as you set it up",
        per_machine: false,
        kind: Kind::Setup,
        only: None,
    },
    Known {
        path: ".vimrc",
        scope: "personal",
        why: "vim, as you set it up",
        per_machine: false,
        kind: Kind::Setup,
        only: None,
    },
    Known {
        path: ".editorconfig",
        scope: "personal",
        why: "editor defaults",
        per_machine: false,
        kind: Kind::Setup,
        only: None,
    },
    Known {
        path: ".config/starship.toml",
        scope: "personal",
        why: "your prompt",
        per_machine: false,
        kind: Kind::Setup,
        only: None,
    },
    Known {
        path: ".config/ghostty/config",
        scope: "personal",
        why: "your terminal",
        per_machine: false,
        kind: Kind::Setup,
        only: None,
    },
    Known {
        path: ".config/nvim/*",
        scope: "personal",
        why: "your editor's own configuration",
        per_machine: false,
        kind: Kind::Setup,
        only: None,
    },
    // The two that need a filter. Both hold what somebody wrote *and* what a
    // package manager installed, and only the first is worth a store: the
    // second is a binary built for one architecture, which is exactly what
    // the `programs` list exists to carry as a name instead.
    Known {
        path: "bin/*",
        scope: "personal",
        why: "scripts you wrote",
        per_machine: false,
        kind: Kind::Setup,
        only: Some("scripts"),
    },
    Known {
        path: ".local/bin/*",
        scope: "personal",
        why: "scripts you wrote (installed binaries are left out)",
        per_machine: false,
        kind: Kind::Setup,
        only: Some("scripts"),
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
    /// See [`Known::per_machine`].
    pub per_machine: bool,
    /// See [`Known::kind`].
    pub kind: Kind,
    /// See [`Known::only`].
    pub only: Option<&'static str>,
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
        let mut out = format!("[[track]]\npath = \"{shown}\"\n");
        if self.scope != "auto" {
            out.push_str(&format!("scope = \"{}\"\n", self.scope));
        }
        if self.per_machine {
            out.push_str("per_machine = true\n");
        }
        if let Some(only) = self.only {
            out.push_str(&format!("only = \"{only}\"\n"));
        }
        out
    }
}

/// What a scan came to: what to propose, and what it deliberately did not.
#[derive(Debug, Default)]
pub struct Scan {
    pub found: Vec<Finding>,
    /// Paths a git repository is already keeping, with the repository. Not
    /// proposed — whatever keeps that repository keeps these — but reported,
    /// because a shell profile missing from the list otherwise looks like a
    /// bug rather than an answer.
    pub in_git: Vec<(PathBuf, PathBuf)>,
}

/// The repository keeping this path, if one is.
///
/// A settings repository usually symlinks `~/.zshrc` to a file inside itself,
/// so the question is asked of what the link points at rather than the link.
/// The walk stops at the home directory — except when the home *is* the
/// repository, which some people do on purpose.
pub fn kept_by_git(path: &Path) -> Option<PathBuf> {
    let real = std::fs::canonicalize(path).ok()?;
    let mut at = real.parent()?.to_path_buf();
    loop {
        if at.join(".git").exists() {
            return Some(at);
        }
        at = at.parent()?.to_path_buf();
    }
}

/// Everything worth proposing, minus what is already tracked or dismissed.
pub fn scan(home: &Path, tracked: &[PathBuf], dismissed: &[PathBuf]) -> Scan {
    let mut found: Vec<Finding> = Vec::new();
    let mut in_git: Vec<(PathBuf, PathBuf)> = Vec::new();

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
            // A `bin` directory holding nothing but installed binaries is not
            // news: the filter would take none of them, and proposing a track
            // that collects nothing is a way of wasting somebody's attention.
            let mut matches = matches;
            if known.only == Some("scripts") {
                matches.retain(|m| starts_with_shebang(m));
                if matches.is_empty() {
                    continue;
                }
            }
            // Asked of every match, not of the first one. A directory where
            // one script is a link into a repository and the next is not is
            // the ordinary case, and dropping the whole pattern because of
            // the first would take the second with it, silently.
            let mut loose: Vec<PathBuf> = Vec::new();
            for m in matches {
                match kept_by_git(&m) {
                    Some(repo) => in_git.push((m, repo)),
                    None => loose.push(m),
                }
            }
            if loose.is_empty() {
                continue;
            }
            let matches = loose;
            push(
                &mut found,
                Finding {
                    path: matches[0].clone(),
                    pattern: Some(known.path.to_string()),
                    scope: known.scope.to_string(),
                    why: known.why.to_string(),
                    per_machine: known.per_machine,
                    kind: known.kind,
                    only: known.only,
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
                    per_machine: known.per_machine,
                    kind: known.kind,
                    only: known.only,
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
                // Nothing noticed by looking can be known to belong to the
                // machine rather than its owner. Guessing that would name
                // items after a machine that has no business owning them.
                per_machine: false,
                // The heuristic looks for values that read like credentials,
                // so anything it turns up is one by construction.
                kind: Kind::Secret,
                only: None,
                source: Source::Noticed,
            },
        );
    }

    found.retain(|f| !tracked.contains(&f.path) && !dismissed.contains(&f.path));
    found.sort_by(|a, b| a.path.cmp(&b.path));

    // A file a git repository already holds does not need a second keeper,
    // and a settings repository is exactly the arrangement that puts one
    // there: `~/.zshrc` is a link into it. Reported rather than dropped —
    // silence would read as a bug, since the file is plainly there.
    found.retain(|f| match kept_by_git(&f.path) {
        Some(repo) => {
            in_git.push((f.path.clone(), repo));
            false
        }
        None => true,
    });
    in_git.sort();

    Scan { found, in_git }
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

/// What separates a script somebody wrote from a binary a package manager
/// installed. Kept in step with `kitbag_core::collect`, which asks the same
/// question when the track actually collects.
fn starts_with_shebang(path: &Path) -> bool {
    use std::io::Read;
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let mut head = [0u8; 2];
    file.read_exact(&mut head).is_ok() && &head == b"#!"
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

        let found = scan(h.path(), &[], &[]).found;
        let paths: Vec<_> = found.iter().map(|f| f.path.clone()).collect();

        assert!(paths.contains(&h.path().join(".aws/credentials")));
        assert!(paths.contains(&h.path().join(".npmrc")));
    }

    #[test]
    fn a_guess_at_the_scope_comes_with_a_reason() {
        let h = tempfile::tempdir().unwrap();
        write(h.path(), ".aws/config", "[profile x]\n", 0o600);

        let found = scan(h.path(), &[], &[]).found;
        let aws = found.iter().find(|f| f.path.ends_with("config")).unwrap();

        assert_eq!(aws.scope, "work");
        assert!(aws.why.contains("AWS"), "{}", aws.why);
        assert_eq!(aws.source, Source::Catalogue);
    }

    #[test]
    fn a_file_nobody_catalogued_is_still_noticed() {
        let h = tempfile::tempdir().unwrap();
        write(h.path(), ".invented-on-a-tuesday", &secretish(), 0o600);

        let found = scan(h.path(), &[], &[]).found;
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

        let found = scan(h.path(), &[], &[]).found;
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

        let found = scan(h.path(), &[], &[]).found;
        assert!(!found.iter().any(|f| f.path.ends_with(".just-settings")));
    }

    #[test]
    fn a_directory_is_proposed_as_one_pattern_not_a_file_at_a_time() {
        let h = tempfile::tempdir().unwrap();
        write(h.path(), ".envs/a.env", "A=1\n", 0o600);
        write(h.path(), ".envs/b.env", "B=2\n", 0o600);

        let found = scan(h.path(), &[], &[]).found;
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

        let found = scan(h.path(), std::slice::from_ref(&a), &[]).found;
        assert!(!found.iter().any(|f| f.pattern.is_some()));
    }

    #[test]
    fn what_is_already_tracked_is_not_proposed_again() {
        let h = tempfile::tempdir().unwrap();
        let npmrc = write(h.path(), ".npmrc", "registry=x\n", 0o600);

        let found = scan(h.path(), std::slice::from_ref(&npmrc), &[]).found;
        assert!(!found.iter().any(|f| f.path == npmrc));
    }

    #[test]
    fn a_dismissal_is_remembered() {
        let h = tempfile::tempdir().unwrap();
        let path = write(h.path(), ".pypirc", "[pypi]\n", 0o600);

        let found = scan(h.path(), &[], std::slice::from_ref(&path)).found;
        assert!(
            !found.iter().any(|f| f.path == path),
            "the second run is quiet"
        );
    }

    #[test]
    fn a_finding_prints_the_lines_to_paste() {
        let h = tempfile::tempdir().unwrap();
        write(h.path(), ".aws/config", "[profile x]\n", 0o600);

        let found = scan(h.path(), &[], &[]).found;
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
                per_machine: false,
                kind: Kind::Secret,
                only: None,
                source: Source::Catalogue,
            }
            .as_toml(h.path()),
            "[[track]]\npath = \"~/.envs/x.env\"\n"
        );
    }
}

#[cfg(test)]
mod per_machine_tests {
    use super::*;

    fn key_entry() -> &'static Known {
        CATALOGUE
            .iter()
            .find(|k| k.path == ".ssh/id_*")
            .expect("the catalogue knows about private keys")
    }

    #[test]
    fn a_private_key_is_proposed_as_this_machines_own() {
        // Four machines keep a key at the same path and they are all
        // different. One item name for four of them leaves three backed up
        // nowhere, which is the whole failure this tool exists to avoid — and
        // it is not something a person should have to know to ask for.
        assert!(key_entry().per_machine);
        let home = tempfile::tempdir().expect("a home");
        let f = Finding {
            path: home.path().join(".ssh/id_ed25519"),
            pattern: Some(".ssh/id_*".into()),
            scope: "personal".into(),
            why: key_entry().why.into(),
            per_machine: true,
            kind: Kind::Secret,
            only: None,
            source: Source::Catalogue,
        };
        let toml = f.as_toml(home.path());
        assert!(toml.contains("per_machine = true"), "{toml}");
        assert!(toml.contains("scope = \"personal\""), "{toml}");
    }

    #[test]
    fn nothing_else_in_the_catalogue_claims_to_belong_to_a_machine() {
        // An item named after a machine is one no other machine will take.
        // That is right for a key and wrong for everything a person owns.
        let named: Vec<&str> = CATALOGUE
            .iter()
            .filter(|k| k.per_machine)
            .map(|k| k.path)
            .collect();
        assert_eq!(named, vec![".ssh/id_*"]);
    }

    #[test]
    fn a_finding_that_belongs_to_nobody_in_particular_says_nothing_about_machines() {
        let home = tempfile::tempdir().expect("a home");
        let f = Finding {
            path: home.path().join(".aws/credentials"),
            pattern: None,
            scope: "work".into(),
            why: "AWS access keys".into(),
            per_machine: false,
            kind: Kind::Secret,
            only: None,
            source: Source::Catalogue,
        };
        assert!(!f.as_toml(home.path()).contains("per_machine"));
    }
}

#[cfg(test)]
mod setup_tests {
    use super::*;

    #[test]
    fn a_bin_directory_offers_the_scripts_and_not_the_installed_binaries() {
        // This is the whole reason the filter exists. `~/.local/bin` holds
        // kitbag itself; a store that took it would be keeping a binary built
        // for one architecture — the thing the `programs` list exists to
        // carry as a name instead.
        let home = tempfile::tempdir().expect("a home");
        std::fs::create_dir_all(home.path().join(".local/bin")).unwrap();
        std::fs::write(home.path().join(".local/bin/mine"), "#!/bin/sh\necho hi\n").unwrap();
        std::fs::write(
            home.path().join(".local/bin/installed"),
            b"\x7fELF\x02\x01\x01\0",
        )
        .unwrap();

        let found = scan(home.path(), &[], &[]).found;
        let bin: Vec<&Finding> = found
            .iter()
            .filter(|f| f.pattern.as_deref() == Some(".local/bin/*"))
            .collect();
        assert_eq!(bin.len(), 1, "{found:#?}");
        assert_eq!(bin[0].only, Some("scripts"));
        assert!(bin[0].as_toml(home.path()).contains("only = \"scripts\""));
    }

    #[test]
    fn a_bin_directory_of_nothing_but_binaries_is_not_news() {
        // Proposing a track that would collect nothing spends somebody's
        // attention for no return.
        let home = tempfile::tempdir().expect("a home");
        std::fs::create_dir_all(home.path().join("bin")).unwrap();
        std::fs::write(home.path().join("bin/installed"), b"\x7fELF\x02\x01\x01\0").unwrap();

        let found = scan(home.path(), &[], &[]).found;
        assert!(
            !found.iter().any(|f| f.pattern.as_deref() == Some("bin/*")),
            "{found:#?}"
        );
    }

    #[test]
    fn the_catalogue_covers_more_than_credentials() {
        // A machine that came back with every secret intact and no shell
        // profile is a machine somebody still has to spend an evening on.
        let setup: Vec<&str> = CATALOGUE
            .iter()
            .filter(|k| k.kind == Kind::Setup)
            .map(|k| k.path)
            .collect();
        for expected in [".zshrc", "bin/*", ".local/bin/*", ".config/nvim/*"] {
            assert!(setup.contains(&expected), "{expected} is not in {setup:?}");
        }
        assert!(CATALOGUE.iter().any(|k| k.kind == Kind::Secret));
    }

    #[test]
    fn a_shell_profile_is_offered_and_says_what_it_is() {
        let home = tempfile::tempdir().expect("a home");
        std::fs::write(home.path().join(".zshrc"), "alias k=kubectl\n").unwrap();
        let found = scan(home.path(), &[], &[]).found;
        let rc = found
            .iter()
            .find(|f| f.shown(home.path()) == "~/.zshrc")
            .expect("the shell profile is proposed");
        assert_eq!(rc.kind, Kind::Setup);
        assert_eq!(rc.scope, "personal");
        assert!(!rc.as_toml(home.path()).contains("only"));
    }
}

#[cfg(test)]
mod git_tests {
    use super::*;

    /// A settings repository, and a home whose shell profile links into it.
    fn home_linked_to_a_repo() -> tempfile::TempDir {
        let home = tempfile::tempdir().expect("a home");
        let repo = home.path().join("workspace/settings");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        std::fs::create_dir_all(repo.join("configs")).unwrap();
        std::fs::write(repo.join("configs/.zshrc"), "alias k=kubectl\n").unwrap();
        std::os::unix::fs::symlink(repo.join("configs/.zshrc"), home.path().join(".zshrc"))
            .unwrap();
        // And one that is nobody's but this machine's.
        std::fs::write(home.path().join(".tmux.conf"), "set -g prefix C-a\n").unwrap();
        home
    }

    #[test]
    fn a_file_a_repository_already_holds_is_not_offered_twice() {
        let scanned = scan(home_linked_to_a_repo().path(), &[], &[]);
        assert!(
            !scanned.found.iter().any(|f| f.path.ends_with(".zshrc")),
            "{:#?}",
            scanned.found
        );
        assert!(scanned.found.iter().any(|f| f.path.ends_with(".tmux.conf")));
    }

    #[test]
    fn and_it_says_so_rather_than_going_quiet() {
        // A shell profile plainly sitting there and missing from the list
        // reads as a bug. Saying which repository holds it is the difference
        // between an answer and a silence.
        let home = home_linked_to_a_repo();
        let scanned = scan(home.path(), &[], &[]);
        assert_eq!(scanned.in_git.len(), 1, "{:#?}", scanned.in_git);
        let (path, repo) = &scanned.in_git[0];
        assert!(path.ends_with(".zshrc"), "{path:?}");
        assert!(repo.ends_with("workspace/settings"), "{repo:?}");
    }

    #[test]
    fn a_file_under_no_repository_at_all_is_still_offered() {
        let home = tempfile::tempdir().expect("a home");
        std::fs::write(home.path().join(".zshrc"), "alias k=kubectl\n").unwrap();
        let scanned = scan(home.path(), &[], &[]);
        assert!(scanned.in_git.is_empty());
        assert!(scanned.found.iter().any(|f| f.path.ends_with(".zshrc")));
    }
}
