//! What this machine tracks, and which scopes it takes.
//!
//! Two files, split by who may read them. `kitbag.toml` in a repository holds
//! recipes and patterns and is meant to be public: it says `~/.envs/*.env`,
//! never which of those exist. `~/.config/kitbag/machine.toml` is this
//! machine's own and may name paths that only exist here.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::scope::{Scope, Wanted};

#[derive(Debug, Default, Deserialize)]
pub struct Config {
    /// Scopes this machine is willing to hold. Absent means personal only.
    #[serde(default)]
    pub scopes: Vec<String>,
    #[serde(default, rename = "track")]
    pub tracks: Vec<Track>,
}

/// State an application keeps for itself, reachable only through the
/// application: an account bundle, an OTP vault, a browser profile.
///
/// `export` writes it to stdout; `restore` reads it from stdin. The tool that
/// owns the data is the only thing that knows how to hand it over and how to
/// take it back, so kitbag asks it rather than copying files out from under it.
#[derive(Debug, Clone, Deserialize)]
pub struct Exported {
    pub export: String,
    pub restore: String,
}

/// One thing to keep: a file, a set of files, or a command pair.
#[derive(Debug, Clone, Deserialize)]
pub struct Track {
    /// A path or a glob, `~` expanded. Absent for a command-sourced item.
    #[serde(default)]
    pub path: Option<String>,
    /// `auto` (the default) reads the marker out of each file and refuses a
    /// file that carries none. Anything else is the scope for every file the
    /// pattern matches — but a file's own marker still wins.
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub owner: Option<String>,
    /// Overrides the name derived from the path. Only meaningful for a single
    /// file; a glob names each match after its own path.
    #[serde(default)]
    pub name: Option<String>,
    /// For state that is not a file. Needs a name, since there is no path to
    /// take one from.
    #[serde(default)]
    pub command: Option<Exported>,
    /// What a `mixed` item is mixed from, for the report.
    #[serde(default)]
    pub spans: Vec<String>,
    /// The platforms this belongs on: `["macos"]` for a keychain or a bundle
    /// addressed to `~/Library`. Empty means everywhere, which is most things.
    /// It is recorded with the item, so the machine that has to act on it —
    /// the one being restored — learns it from the store rather than from a
    /// config it does not have yet.
    #[serde(default)]
    pub platform: Vec<String>,
    /// Set when the export is not byte-stable: tokens that rotate on their own,
    /// a timestamp baked into the format. Comparing such an item against the
    /// store answers "different" every time and means nothing by it, so kitbag
    /// stops claiming it changed and reports that it cannot tell.
    #[serde(default)]
    pub volatile: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("{path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
}

impl Config {
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let text = std::fs::read_to_string(path).map_err(|source| ConfigError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        toml::from_str(&text).map_err(|source| ConfigError::Parse {
            path: path.to_path_buf(),
            source,
        })
    }

    /// The machine file if it exists; otherwise a machine that tracks nothing
    /// and takes only personal — which is what an unconfigured machine should
    /// do, rather than fail.
    pub fn load_or_default(path: &Path) -> Result<Self, ConfigError> {
        if path.exists() {
            Self::load(path)
        } else {
            Ok(Self::default())
        }
    }

    pub fn wanted(&self) -> Wanted {
        if self.scopes.is_empty() {
            Wanted::default()
        } else {
            Wanted::parse(&self.scopes.join(",")).unwrap_or_default()
        }
    }
}

impl Track {
    /// The scope declared in the config, if any. A file's own marker still
    /// wins over this: the marker travels with the file, the table does not.
    pub fn declared_scope(&self) -> Option<Scope> {
        match self.scope.as_deref() {
            None | Some("auto") => None,
            Some(other) => match other.parse().ok()? {
                // A `mixed` item has to say what it is mixed from, or the one
                // scope meaning "I could not separate this" hides what is in it.
                Scope::Mixed { .. } => Some(Scope::Mixed {
                    spans: self.spans.clone(),
                }),
                scope => Some(scope),
            },
        }
    }
}

/// `~/x` against a home directory. Anything already absolute is left alone.
pub fn expand(pattern: &str, home: &Path) -> String {
    match pattern.strip_prefix("~/") {
        Some(rest) => home.join(rest).to_string_lossy().into_owned(),
        None => pattern.to_string(),
    }
}

/// The name an item gets from where it lives.
///
/// These are conventions, not rules: `~/.envs/github.env` is `env:github`
/// because that reads better in a report than `file:envs-github.env`. A track
/// entry can always name an item itself.
/// The inverse of [`expand`]: how a path is written down so another machine
/// can place it.
pub fn shorten(path: &std::path::Path, home: &std::path::Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

pub fn derive_name(path: &Path, home: &Path) -> String {
    let rel = path.strip_prefix(home).unwrap_or(path);
    let rel_str = rel.to_string_lossy();

    if let Some(rest) = rel_str.strip_prefix(".envs/") {
        return format!("env:{}", rest.trim_end_matches(".env"));
    }
    if let Some(rest) = rel_str.strip_prefix(".ssh/config.d/") {
        return format!("ssh:config-{}", rest.trim_end_matches(".conf"));
    }
    if let Some(rest) = rel_str.strip_prefix(".ssh/") {
        return format!("ssh:{rest}");
    }
    let slug = rel_str
        .trim_start_matches('.')
        .replace('/', "-")
        .replace("..", "");
    format!("file:{slug}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_machine_that_was_never_configured_takes_only_personal() {
        let cfg = Config::default();
        assert!(cfg.wanted().accepts(&Scope::Personal));
        assert!(!cfg.wanted().accepts(&Scope::Work));
        assert!(cfg.tracks.is_empty());
    }

    #[test]
    fn reads_a_machine_file() {
        let cfg: Config = toml::from_str(
            r#"
            scopes = ["personal", "work"]

            [[track]]
            path = "~/.envs/*.env"

            [[track]]
            path = "~/Library/Keychains/x.keychain-db"
            scope = "work"
            owner = "acme"
            name = "file:x-keychain"
        "#,
        )
        .unwrap();

        assert!(cfg.wanted().accepts(&Scope::Work));
        assert_eq!(cfg.tracks.len(), 2);
        assert_eq!(cfg.tracks[0].declared_scope(), None); // auto: read the file
        assert_eq!(cfg.tracks[1].declared_scope(), Some(Scope::Work));
        assert_eq!(cfg.tracks[1].name.as_deref(), Some("file:x-keychain"));
    }

    #[test]
    fn an_application_can_be_asked_for_its_own_state() {
        let cfg: Config = toml::from_str(
            r#"
            [[track]]
            name = "app:accounts"
            scope = "mixed"
            spans = ["personal", "work"]
            command = { export = "acct export --all", restore = "acct import -" }
        "#,
        )
        .unwrap();

        let t = &cfg.tracks[0];
        assert_eq!(t.name.as_deref(), Some("app:accounts"));
        assert_eq!(t.command.as_ref().unwrap().export, "acct export --all");
        assert_eq!(t.spans, ["personal", "work"]);
        assert!(matches!(t.declared_scope(), Some(Scope::Mixed { .. })));
    }

    #[test]
    fn auto_is_the_same_as_saying_nothing() {
        let t: Track = toml::from_str(
            r#"path = "x"
scope = "auto""#,
        )
        .unwrap();
        assert_eq!(t.declared_scope(), None);
    }

    #[test]
    fn names_come_from_where_a_file_lives() {
        let home = Path::new("/home/user");
        assert_eq!(
            derive_name(&home.join(".envs/github.env"), home),
            "env:github"
        );
        assert_eq!(
            derive_name(&home.join(".ssh/config.d/20-work.conf"), home),
            "ssh:config-20-work"
        );
        assert_eq!(
            derive_name(&home.join(".ssh/id_ed25519"), home),
            "ssh:id_ed25519"
        );
        assert_eq!(derive_name(&home.join(".npmrc"), home), "file:npmrc");
        assert_eq!(
            derive_name(&home.join("Library/Keychains/a.keychain-db"), home),
            "file:Library-Keychains-a.keychain-db"
        );
    }

    #[test]
    fn tilde_is_this_home_and_nothing_else() {
        let home = Path::new("/home/user");
        assert_eq!(expand("~/.envs/a.env", home), "/home/user/.envs/a.env");
        assert_eq!(expand("/etc/hosts", home), "/etc/hosts");
    }
}
