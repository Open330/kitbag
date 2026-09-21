//! Reading what this machine holds.
//!
//! Collection is by pattern, never by name: the config says `~/.envs/*.env`,
//! and what exists there is between the machine and its owner. A file with no
//! usable marker is **skipped and reported**, never guessed at — filing a
//! secret into the wrong life is worse than not filing it.

use std::path::{Path, PathBuf};

use crate::config::{derive_name, expand, Config, Track};
use crate::marker;
use crate::scope::Scope;

/// Where an item's payload came from, and how it goes back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// A file, which is also where a restore writes it.
    File(PathBuf),
    /// An application, asked for its own state. The string is the command that
    /// takes it back.
    Command { restore: String },
}

/// One thing found on the machine, ready to be compared or sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub name: String,
    pub scope: Scope,
    pub owner: Option<String>,
    pub path: PathBuf,
    pub payload: Vec<u8>,
    pub source: Source,
    /// The export is not byte-stable, so comparing it says nothing. See
    /// [`crate::config::Track::volatile`].
    pub volatile: bool,
    /// The platforms this belongs on, empty meaning all of them.
    pub platform: Vec<String>,
    /// Set when this belongs to one machine rather than to their owner, so
    /// that four machines can each keep their own under a name of their own.
    pub machine: Option<String>,
}

impl Item {
    /// What is inside, in the terms of whatever it is: the variable names of an
    /// env file, the size of a binary. Never a value.
    pub fn detail(&self) -> String {
        if let Source::Command { .. } = self.source {
            return format!("{} bytes from a command", self.payload.len());
        }
        match std::str::from_utf8(&self.payload) {
            Ok(text) => {
                let keys = key_names(text);
                if keys.is_empty() {
                    format!(
                        "{} lines",
                        text.lines().filter(|l| !l.trim().is_empty()).count()
                    )
                } else {
                    keys
                }
            }
            Err(_) => format!("binary, {} bytes", self.payload.len()),
        }
    }
}

/// The envelope a push sends for an item.
///
/// One place, so that what is compared against the store and what is written
/// to it cannot describe the item differently — which is how a change to a
/// marker ends up invisible.
pub fn envelope_for(item: &Item, home: &Path) -> crate::Envelope {
    crate::Envelope::new(item.scope.clone(), item.payload.clone())
        .with_owner(item.owner.clone())
        .with_platform(item.platform.clone())
        .with_path(match &item.source {
            // State an application owns has no path: it goes back the way it
            // came out, through the app.
            Source::Command { .. } => None,
            Source::File(path) => Some(crate::config::shorten(path, home)),
        })
        .with_machine(item.machine.clone())
        .with_restore(match &item.source {
            Source::Command { restore } => Some(restore.clone()),
            Source::File(_) => None,
        })
}

/// Why something on disk was passed over.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    pub path: PathBuf,
    pub reason: Reason,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reason {
    /// No `# scope:` marker, and the config did not supply one.
    Unmarked,
    /// A marker, but not one this version knows.
    UnknownScope(String),
    /// Declared machine-local: it stays here by instruction.
    Local,
    /// Listed in the config, absent from the disk. Usually a machine that has
    /// not been set up yet rather than a typo, so it is worth saying.
    Missing,
    /// A command-sourced item with no name: there is no path to take one from.
    Unnamed,
    /// The command ran and said nothing, which is not state worth keeping.
    Empty,
    Unreadable(String),
}

impl Reason {
    pub fn says(&self) -> String {
        match self {
            Reason::Unmarked => "no '# scope:' marker, and the config does not say".into(),
            Reason::UnknownScope(s) => format!("unknown scope `{s}`"),
            Reason::Local => "machine-local by its own marker".into(),
            Reason::Missing => "listed here, but not on this machine".into(),
            Reason::Unnamed => "a command-sourced item needs a name".into(),
            Reason::Empty => "the command produced nothing".into(),
            Reason::Unreadable(e) => format!("could not be read: {e}"),
        }
    }
}

#[derive(Debug, Default)]
pub struct Collected {
    pub items: Vec<Item>,
    pub skipped: Vec<Skipped>,
}

/// Everything the config points at, under `home`.
pub fn collect(config: &Config, home: &Path) -> Collected {
    let mut out = Collected::default();
    let machine = config.machine_name();
    for track in &config.tracks {
        // Where this track's items begin. `out.items` accumulates across
        // tracks, and renaming by walking all of it renamed every item
        // collected so far — twenty-nine env files took an ssh key's rule.
        let mine = out.items.len();
        collect_track(track, home, &mut out);
        // Naming happens after collection, so a glob still derives each file's
        // own name before the machine is added to it.
        if track.per_machine {
            for item in &mut out.items[mine..] {
                item.name = format!("{}@{machine}", item.name);
                item.machine = Some(machine.clone());
            }
        }
    }
    out.items.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

fn collect_track(track: &Track, home: &Path, out: &mut Collected) {
    if let Some(exported) = &track.command {
        collect_command(track, exported, out);
        return;
    }
    let Some(pattern) = track.path.as_deref() else {
        return;
    };
    let expanded = expand(pattern, home);

    let paths: Vec<PathBuf> = if expanded.contains('*') || expanded.contains('?') {
        match glob::glob(&expanded) {
            Ok(entries) => entries.filter_map(Result::ok).collect(),
            Err(_) => Vec::new(),
        }
    } else {
        vec![PathBuf::from(&expanded)]
    };

    if paths.is_empty() && !expanded.contains('*') {
        out.skipped.push(Skipped {
            path: PathBuf::from(expanded),
            reason: Reason::Missing,
        });
        return;
    }

    for path in paths {
        if !path.is_file() {
            if !expanded.contains('*') {
                out.skipped.push(Skipped {
                    path,
                    reason: Reason::Missing,
                });
            }
            continue;
        }
        match read_item(&path, track, home) {
            Ok(item) => out.items.push(item),
            Err(reason) => out.skipped.push(Skipped { path, reason }),
        }
    }
}

/// Ask an application for its own state.
///
/// The payload is whatever the command writes, byte for byte. Whether it is
/// stable between runs is the application's business: one whose output changes
/// every time is sent every time, which is the honest outcome. Mark such a
/// track `volatile` and the report stops calling that difference a change.
fn collect_command(track: &Track, exported: &crate::config::Exported, out: &mut Collected) {
    let name = match &track.name {
        Some(n) => n.clone(),
        None => {
            out.skipped.push(Skipped {
                path: PathBuf::from(&exported.export),
                reason: Reason::Unnamed,
            });
            return;
        }
    };
    let Some(scope) = track.declared_scope() else {
        out.skipped.push(Skipped {
            path: PathBuf::from(&exported.export),
            reason: Reason::Unmarked,
        });
        return;
    };
    if scope == Scope::Local {
        return;
    }

    match std::process::Command::new("sh")
        .arg("-c")
        .arg(&exported.export)
        .output()
    {
        Ok(o) if o.status.success() && !o.stdout.is_empty() => out.items.push(Item {
            name,
            scope,
            owner: track.owner.clone(),
            path: PathBuf::from(&exported.export),
            payload: o.stdout,
            source: Source::Command {
                restore: exported.restore.clone(),
            },
            volatile: track.volatile,
            platform: track.platform.clone(),
            machine: None,
        }),
        Ok(o) if o.status.success() => out.skipped.push(Skipped {
            path: PathBuf::from(&exported.export),
            reason: Reason::Empty,
        }),
        Ok(o) => out.skipped.push(Skipped {
            path: PathBuf::from(&exported.export),
            reason: Reason::Unreadable(
                String::from_utf8_lossy(&o.stderr)
                    .lines()
                    .next()
                    .unwrap_or("the command failed")
                    .to_string(),
            ),
        }),
        Err(e) => out.skipped.push(Skipped {
            path: PathBuf::from(&exported.export),
            reason: Reason::Unreadable(e.to_string()),
        }),
    }
}

fn read_item(path: &Path, track: &Track, home: &Path) -> Result<Item, Reason> {
    let payload = std::fs::read(path).map_err(|e| Reason::Unreadable(e.to_string()))?;

    // The head of the file, if it is text at all: markers live in a comment.
    let head = std::str::from_utf8(&payload).ok().unwrap_or("");
    let markers = match marker::parse(head) {
        Ok(m) => m,
        Err(e) => return Err(Reason::UnknownScope(e.0)),
    };

    // A marker inside the file beats the config, because the marker travels
    // with the file and the config does not.
    let scope = match markers.scope.or_else(|| track.declared_scope()) {
        Some(s) => s,
        None => return Err(Reason::Unmarked),
    };
    if scope == Scope::Local {
        return Err(Reason::Local);
    }

    let name = track
        .name
        .clone()
        .unwrap_or_else(|| derive_name(path, home));

    Ok(Item {
        name,
        scope,
        owner: markers.owner.or_else(|| track.owner.clone()),
        path: path.to_path_buf(),
        payload,
        source: Source::File(path.to_path_buf()),
        volatile: track.volatile,
        platform: track.platform.clone(),
        machine: None,
    })
}

/// The left-hand sides of `key = value` lines, whatever syntax surrounds them:
/// env files, npmrc, ini, toml all answer to this. Never the values.
const KEYS_SHOWN: usize = 12;

/// A name a person chose, as opposed to a piece of somebody's key.
///
/// This is the line that matters. Base64 is full of `=`, so a private key
/// splits into "names" that are the key itself - and printing one in a report
/// is the exact failure this tool exists to avoid. Anything that does not look
/// like an identifier, or that the rule set recognises as a secret, is not a
/// name.
pub(crate) fn is_a_name(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 40
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_-.".contains(c))
        && crate::lint::check(key).is_empty()
}

fn key_names(text: &str) -> String {
    // A key file is not a settings file, whatever its punctuation suggests.
    if text.contains("PRIVATE KEY") || text.starts_with("ssh-") {
        return String::new();
    }

    let mut seen: Vec<String> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        let Some((left, _)) = line.split_once('=') else {
            continue;
        };
        let key = left.trim().trim_start_matches("export ").trim().to_string();
        if !is_a_name(&key) {
            continue;
        }
        if !seen.contains(&key) {
            seen.push(key);
        }
    }
    if seen.len() > KEYS_SHOWN {
        let rest = seen.len() - KEYS_SHOWN;
        format!("{} +{rest} more", seen[..KEYS_SHOWN].join(" "))
    } else {
        seen.join(" ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn home() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    fn write(home: &Path, rel: &str, body: &str) {
        let path = home.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    }

    fn config(toml_src: &str) -> Config {
        toml::from_str(toml_src).unwrap()
    }

    #[test]
    fn a_glob_finds_what_is_there_and_reads_each_marker() {
        let h = home();
        write(h.path(), ".envs/one.env", "# scope: personal\nA=1\n");
        write(
            h.path(),
            ".envs/two.env",
            "# scope: work\n# owner: acme\nB=2\n",
        );

        let got = collect(&config("[[track]]\npath = \"~/.envs/*.env\""), h.path());

        assert_eq!(got.items.len(), 2);
        assert_eq!(got.items[0].name, "env:one");
        assert_eq!(got.items[0].scope, Scope::Personal);
        assert_eq!(got.items[1].scope, Scope::Work);
        assert_eq!(got.items[1].owner.as_deref(), Some("acme"));
    }

    #[test]
    fn an_unmarked_file_is_reported_not_guessed_at() {
        let h = home();
        write(h.path(), ".envs/bare.env", "A=1\n");

        let got = collect(&config("[[track]]\npath = \"~/.envs/*.env\""), h.path());

        assert!(got.items.is_empty());
        assert_eq!(got.skipped.len(), 1);
        assert_eq!(got.skipped[0].reason, Reason::Unmarked);
    }

    #[test]
    fn the_file_beats_the_config() {
        let h = home();
        write(h.path(), ".npmrc", "# scope: work\nregistry=x\n");

        let got = collect(
            &config("[[track]]\npath = \"~/.npmrc\"\nscope = \"personal\""),
            h.path(),
        );

        assert_eq!(
            got.items[0].scope,
            Scope::Work,
            "the marker travels, the table does not"
        );
    }

    #[test]
    fn the_config_supplies_a_scope_for_a_file_that_cannot_carry_one() {
        let h = home();
        fs::create_dir_all(h.path().join("Library/Keychains")).unwrap();
        fs::write(
            h.path().join("Library/Keychains/x.keychain-db"),
            [0u8, 1, 2, 255],
        )
        .unwrap();

        let got = collect(
            &config(
                "[[track]]\npath = \"~/Library/Keychains/x.keychain-db\"\nscope = \"work\"\nname = \"file:x\"",
            ),
            h.path(),
        );

        assert_eq!(got.items.len(), 1);
        assert_eq!(got.items[0].name, "file:x");
        assert_eq!(got.items[0].scope, Scope::Work);
        assert_eq!(got.items[0].detail(), "binary, 4 bytes");
    }

    #[test]
    fn machine_local_stays_here() {
        let h = home();
        write(h.path(), ".envs/session.env", "# scope: local\nS=1\n");

        let got = collect(&config("[[track]]\npath = \"~/.envs/*.env\""), h.path());

        assert!(got.items.is_empty());
        assert_eq!(got.skipped[0].reason, Reason::Local);
    }

    #[test]
    fn a_listed_path_that_is_not_here_is_said_out_loud() {
        let h = home();
        let got = collect(
            &config("[[track]]\npath = \"~/.pypirc\"\nscope = \"personal\""),
            h.path(),
        );
        assert_eq!(got.skipped[0].reason, Reason::Missing);
    }

    #[test]
    fn an_application_is_asked_for_its_own_state() {
        let h = home();
        let got = collect(
            &config(
                r#"
                [[track]]
                name = "app:accounts"
                scope = "mixed"
                spans = ["personal", "work"]
                command = { export = "printf 'bundle'", restore = "acct import -" }
            "#,
            ),
            h.path(),
        );

        assert_eq!(got.items.len(), 1);
        assert_eq!(got.items[0].name, "app:accounts");
        assert_eq!(got.items[0].payload, b"bundle");
        assert_eq!(
            got.items[0].source,
            Source::Command {
                restore: "acct import -".into()
            }
        );
        assert!(matches!(got.items[0].scope, Scope::Mixed { .. }));
    }

    #[test]
    fn a_command_that_says_nothing_is_not_state() {
        let h = home();
        let got = collect(
            &config(
                r#"
                [[track]]
                name = "app:nothing"
                scope = "personal"
                command = { export = "true", restore = "cat" }
            "#,
            ),
            h.path(),
        );
        assert!(got.items.is_empty());
        assert_eq!(got.skipped[0].reason, Reason::Empty);
    }

    #[test]
    fn a_command_that_fails_says_what_it_said() {
        let h = home();
        let got = collect(
            &config(
                r#"
                [[track]]
                name = "app:broken"
                scope = "personal"
                command = { export = "echo 'not logged in' >&2; exit 1", restore = "cat" }
            "#,
            ),
            h.path(),
        );
        match &got.skipped[0].reason {
            Reason::Unreadable(why) => assert!(why.contains("not logged in"), "{why}"),
            other => panic!("expected the reason, got {other:?}"),
        }
    }

    #[test]
    fn a_command_with_no_name_has_nowhere_to_be_filed() {
        let h = home();
        let got = collect(
            &config(
                r#"
                [[track]]
                scope = "personal"
                command = { export = "echo x", restore = "cat" }
            "#,
            ),
            h.path(),
        );
        assert_eq!(got.skipped[0].reason, Reason::Unnamed);
    }

    #[test]
    fn detail_names_the_keys_and_never_the_values() {
        let h = home();
        write(
            h.path(),
            ".envs/db.env",
            "# scope: personal\nexport DB_URL=postgres://secret\nDB_USER=admin\n",
        );

        let got = collect(&config("[[track]]\npath = \"~/.envs/*.env\""), h.path());
        let detail = got.items[0].detail();

        assert_eq!(detail, "DB_URL DB_USER");
        assert!(!detail.contains("postgres"));
        assert!(!detail.contains("admin"));
    }

    #[test]
    fn a_private_key_never_shows_any_of_itself() {
        let h = home();
        // The shape that caused this: base64 is full of `=`, so a key file
        // splits into "names" that are the key.
        let header = format!("-----{} OPENSSH PRIVATE KEY-----", "BEGIN"); // lint:allow
        let body = format!(
            "{header}\n{}{}{}=\n{}\n",
            "b3BlbnNzaC1rZXkt",
            "djEAAAAABG5vbmUA",
            "AAAEbm9uZQAAAAtz",
            header.replace("BEGIN", "END")
        );
        write(h.path(), ".ssh/id_ed25519", &body);

        let got = collect(
            &config("[[track]]\npath = \"~/.ssh/id_ed25519\"\nscope = \"personal\""),
            h.path(),
        );
        let detail = got.items[0].detail();

        assert!(detail.ends_with("lines"), "{detail}");
        for fragment in ["b3Blbn", "djEAAA", "bm9uZQ"] {
            assert!(!detail.contains(fragment), "the key leaked into {detail}");
        }
    }

    #[test]
    fn a_public_key_file_shows_no_key_material_either() {
        let h = home();
        write(
            h.path(),
            ".ssh/authorized_keys",
            &format!(
                "# scope: personal\nssh-ed25519 {}{} laptop\n",
                "AAAAC3NzaC1lZDI1", "NTE5AAAAIabcdefgh"
            ),
        );
        let got = collect(
            &config("[[track]]\npath = \"~/.ssh/authorized_keys\""),
            h.path(),
        );
        assert!(
            !got.items[0].detail().contains("AAAA"),
            "{}",
            got.items[0].detail()
        );
    }

    #[test]
    fn a_long_key_list_is_cut_short() {
        let h = home();
        let body: String = (0..20).map(|i| format!("KEY_{i}=v\n")).collect();
        write(
            h.path(),
            ".envs/big.env",
            &format!("# scope: personal\n{body}"),
        );

        let got = collect(&config("[[track]]\npath = \"~/.envs/*.env\""), h.path());
        assert!(
            got.items[0].detail().ends_with("+8 more"),
            "{}",
            got.items[0].detail()
        );
    }
}
