//! What is installed here, written down so it can be installed again.
//!
//! Not the programs. A binary is the one thing a store should never hold: it
//! is large, it is signed for one architecture, and it is already published
//! somewhere that will hand it over again. What is worth keeping is the list —
//! what was installed, by which manager, at which version where the manager
//! records one.
//!
//! Restoring installs what is missing and never removes what is not listed.
//! A machine is allowed to have more than the list; the list is what it must
//! not be missing.

use std::process::Command;

/// One thing a manager installed.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Program {
    pub manager: String,
    pub name: String,
    /// Empty when the manager does not pin one, which is most of them: a
    /// formula is whatever the tap has today, and recording a version that
    /// cannot be asked for is a fact nobody can act on.
    pub version: String,
}

/// The header a written list starts with, so a reader knows what it has.
pub const MAGIC: &str = "kitbag/programs 1";

fn run(program: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(program).args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).to_string())
}

fn have(program: &str) -> bool {
    Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {program} >/dev/null 2>&1"))
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Everything the managers on this machine will admit to.
pub fn installed() -> Vec<Program> {
    let mut out = Vec::new();

    if have("brew") {
        // `leaves` and not `list`: what was asked for, rather than that plus
        // everything pulled in behind it. Reinstalling the first gets the
        // second for free; reinstalling the second pins things nobody chose.
        if let Some(text) = run("brew", &["leaves", "--installed-on-request"]) {
            for name in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
                out.push(Program {
                    manager: "brew".into(),
                    name: name.into(),
                    version: String::new(),
                });
            }
        }
        if let Some(text) = run("brew", &["list", "--cask"]) {
            for name in text.split_whitespace().filter(|l| !l.is_empty()) {
                out.push(Program {
                    manager: "cask".into(),
                    name: name.into(),
                    version: String::new(),
                });
            }
        }
    }

    if have("cargo") {
        // `name vX.Y.Z:` — the version is the one thing cargo will install
        // again on request, so it is worth keeping.
        if let Some(text) = run("cargo", &["install", "--list"]) {
            for line in text.lines() {
                if line.starts_with(char::is_whitespace) {
                    continue;
                }
                let line = line.trim_end_matches(':');
                if let Some((name, version)) = line.split_once(' ') {
                    out.push(Program {
                        manager: "cargo".into(),
                        name: name.into(),
                        version: version.trim_start_matches('v').into(),
                    });
                }
            }
        }
    }

    if have("npm") {
        if let Some(text) = run("npm", &["ls", "-g", "--depth=0", "--json"]) {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                if let Some(deps) = json.get("dependencies").and_then(|d| d.as_object()) {
                    for (name, body) in deps {
                        out.push(Program {
                            manager: "npm".into(),
                            name: name.clone(),
                            version: body
                                .get("version")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default()
                                .to_string(),
                        });
                    }
                }
            }
        }
    }

    if have("rustup") {
        if let Some(text) = run("rustup", &["toolchain", "list"]) {
            for line in text.lines() {
                let name = line.split_whitespace().next().unwrap_or("");
                if !name.is_empty() {
                    out.push(Program {
                        manager: "rustup".into(),
                        name: name.into(),
                        version: String::new(),
                    });
                }
            }
        }
    }

    // Sorted, because an unordered list differs from itself between two runs
    // and a store would report a machine as changed for having listed the
    // same things in another order.
    out.sort();
    out.dedup();
    out
}

/// The list, as it is stored.
pub fn write(programs: &[Program]) -> String {
    let mut out = String::from(MAGIC);
    out.push('\n');
    out.push_str("# what is installed here, so it can be installed again\n");
    for p in programs {
        if p.version.is_empty() {
            out.push_str(&format!("{}\t{}\n", p.manager, p.name));
        } else {
            out.push_str(&format!("{}\t{}\t{}\n", p.manager, p.name, p.version));
        }
    }
    out
}

/// The list, as it was stored. Unknown lines are skipped rather than guessed
/// at: a newer writer may have managers this reader does not know.
pub fn read(text: &str) -> Vec<Program> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line == MAGIC {
            continue;
        }
        let mut parts = line.split('\t');
        let (Some(manager), Some(name)) = (parts.next(), parts.next()) else {
            continue;
        };
        if manager.is_empty() || name.is_empty() {
            continue;
        }
        out.push(Program {
            manager: manager.to_string(),
            name: name.to_string(),
            version: parts.next().unwrap_or("").to_string(),
        });
    }
    out
}

/// What installing one would run. `None` for a manager this does not know how
/// to drive — saying so beats running something plausible.
pub fn install_command(p: &Program) -> Option<Vec<String>> {
    let argv = match p.manager.as_str() {
        "brew" => vec!["brew".into(), "install".into(), p.name.clone()],
        "cask" => vec![
            "brew".into(),
            "install".into(),
            "--cask".into(),
            p.name.clone(),
        ],
        "cargo" => {
            let mut v = vec!["cargo".into(), "install".into()];
            if !p.version.is_empty() {
                v.push("--version".into());
                v.push(p.version.clone());
            }
            v.push(p.name.clone());
            v
        }
        "npm" => {
            let spec = if p.version.is_empty() {
                p.name.clone()
            } else {
                format!("{}@{}", p.name, p.version)
            };
            vec!["npm".into(), "install".into(), "-g".into(), spec]
        }
        "rustup" => vec![
            "rustup".into(),
            "toolchain".into(),
            "install".into(),
            p.name.clone(),
        ],
        _ => return None,
    };
    Some(argv)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(manager: &str, name: &str, version: &str) -> Program {
        Program {
            manager: manager.into(),
            name: name.into(),
            version: version.into(),
        }
    }

    #[test]
    fn what_is_written_comes_back() {
        let list = vec![
            p("brew", "ripgrep", ""),
            p("cargo", "kitbag", "0.11.0"),
            p("npm", "@scope/tool", "1.2.3"),
        ];
        assert_eq!(read(&write(&list)), list);
    }

    #[test]
    fn a_manager_this_reader_does_not_know_is_skipped_not_guessed_at() {
        // A newer writer may list managers this reader has never heard of.
        // Reading them as something else would install the wrong thing.
        let text = format!("{MAGIC}\nbrew\tripgrep\nsnap\tsomething\n");
        let back = read(&text);
        assert_eq!(back.len(), 2);
        assert!(install_command(&back[0]).is_some());
        assert!(
            install_command(&back[1]).is_none(),
            "an unknown manager has no command, and saying so beats a plausible one"
        );
    }

    #[test]
    fn a_version_is_asked_for_when_there_is_one() {
        let with = install_command(&p("cargo", "thing", "1.2.3")).unwrap();
        assert!(
            with.windows(2).any(|w| w == ["--version", "1.2.3"]),
            "{with:?}"
        );

        let without = install_command(&p("cargo", "thing", "")).unwrap();
        assert!(
            !without.iter().any(|a| a == "--version"),
            "a version nobody recorded is not one to ask for: {without:?}"
        );

        // npm spells it differently, and the name must survive a scope.
        let npm = install_command(&p("npm", "@bitwarden/cli", "2026.8.0")).unwrap();
        assert!(
            npm.contains(&"@bitwarden/cli@2026.8.0".to_string()),
            "{npm:?}"
        );
    }

    #[test]
    fn a_cask_is_not_a_formula() {
        let cask = install_command(&p("cask", "ghostty", "")).unwrap();
        assert!(cask.contains(&"--cask".to_string()), "{cask:?}");
    }

    #[test]
    fn the_list_is_sorted_so_it_does_not_differ_from_itself() {
        // Two runs listing the same things in another order would read as a
        // machine that changed.
        let mut a = vec![p("npm", "b", ""), p("brew", "a", ""), p("brew", "b", "")];
        a.sort();
        let mut b = vec![p("brew", "b", ""), p("npm", "b", ""), p("brew", "a", "")];
        b.sort();
        assert_eq!(write(&a), write(&b));
    }

    #[test]
    fn comments_and_the_header_are_not_programs() {
        let back = read(&format!("{MAGIC}\n# a note\n\nbrew\tripgrep\n"));
        assert_eq!(back, vec![p("brew", "ripgrep", "")]);
    }
}
