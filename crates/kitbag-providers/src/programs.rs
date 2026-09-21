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

use kitbag_core::exec::{machine_path, machine_shell};

/// One thing a manager installed.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Program {
    pub manager: String,
    pub name: String,
    /// Empty when the manager does not pin one, which is most of them: a
    /// formula is whatever the tap has today, and recording a version that
    /// cannot be asked for is a fact nobody can act on.
    pub version: String,
    /// How to tell it is already here, when `command -v <name>` is not the
    /// answer. It travels with the list because the machine that has to ask
    /// the question is the one being restored, and it did not declare this
    /// program — without it, a program whose command is not its name gets
    /// reinstalled on every restore.
    pub present: String,
    /// The shell line that installs it, for the things no manager here will
    /// ever list — `rustup`, `uv`, anything that arrives as `curl … | sh`.
    /// Empty for everything a manager can be asked about, which is most of it.
    ///
    /// Written last, and read as the whole rest of the line: a shell line may
    /// hold a tab, and splitting on one would cut the command in half.
    pub install: String,
}

impl Program {
    /// The test for whether this is already installed.
    pub fn presence_test(&self) -> String {
        if self.present.is_empty() {
            format!("command -v {} >/dev/null 2>&1", self.name)
        } else {
            self.present.clone()
        }
    }
}

/// The header a written list starts with, so a reader knows what it has.
///
/// Still `1` with the install column added, because a reader that does not
/// know about it skips the whole line: those lines say `script`, and a manager
/// nobody recognises has always been skipped rather than guessed at.
pub const MAGIC: &str = "kitbag/programs 1";

/// The manager of last resort: the line to run is the record.
pub const SCRIPT: &str = "script";

fn run(program: &str, args: &[&str]) -> Option<String> {
    // With a PATH that knows where a user's own programs live: what is
    // installed on this machine is not a fact about the caller's shell.
    let out = Command::new(program)
        .args(args)
        .env("PATH", machine_path())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).to_string())
}

fn have(program: &str) -> bool {
    machine_shell(&format!("command -v {program} >/dev/null 2>&1"))
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
                    present: String::new(),
                    install: String::new(),
                });
            }
        }
        if let Some(text) = run("brew", &["list", "--cask"]) {
            for name in text.split_whitespace().filter(|l| !l.is_empty()) {
                out.push(Program {
                    manager: "cask".into(),
                    name: name.into(),
                    version: String::new(),
                    present: String::new(),
                    install: String::new(),
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
                        present: String::new(),
                        install: String::new(),
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
                            present: String::new(),
                            install: String::new(),
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
                        present: String::new(),
                        install: String::new(),
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
        // Tab-separated, and a trailing empty field is not written: the common
        // line is `brew<TAB>ripgrep` and it should stay that short.
        if !p.install.is_empty() {
            out.push_str(&format!(
                "{}\t{}\t{}\t{}\t{}\n",
                p.manager, p.name, p.version, p.present, p.install
            ));
        } else if p.version.is_empty() {
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
            present: parts.next().unwrap_or("").to_string(),
            // The rest of the line, tabs and all: a shell line may hold one,
            // and splitting it further would quietly cut the command in half.
            install: parts.collect::<Vec<_>>().join("\t"),
        });
    }
    out
}

/// What installing one would run. `None` for a manager this does not know how
/// to drive — saying so beats running something plausible.
pub fn install_command(p: &Program) -> Option<Vec<String>> {
    if p.manager == SCRIPT {
        // Not argv: a declared line is shell, pipes and all. It is printed
        // before it runs, because a command out of the store is still a
        // command out of the store.
        if p.install.is_empty() {
            return None;
        }
        return Some(vec!["sh".into(), "-c".into(), p.install.clone()]);
    }
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

/// A program the machine declared, as it belongs in the list — and only if it
/// is actually here. The list says what is installed; a declaration that has
/// never been acted on is a plan, not a fact.
pub fn declared(
    name: &str,
    install: &str,
    version_from: Option<&str>,
    present: Option<&str>,
) -> Option<Program> {
    let p = Program {
        manager: SCRIPT.to_string(),
        name: name.to_string(),
        version: String::new(),
        present: present.unwrap_or_default().to_string(),
        install: install.to_string(),
    };
    if !is_here(&p) {
        return None;
    }
    Some(Program {
        version: version_from.and_then(version_in_output).unwrap_or_default(),
        ..p
    })
}

/// Is it already on this machine? Asked with whatever test the list carries,
/// and `command -v <name>` when it carries none.
pub fn is_here(p: &Program) -> bool {
    machine_shell(&p.presence_test())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Run a command and take the first thing on its output that looks like a
/// version. `uv --version` says `uv 0.4.9`; the interesting half is the
/// second word, and which word that is differs per program.
fn version_in_output(command: &str) -> Option<String> {
    let out = machine_shell(command).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    text.split_whitespace()
        .map(|w| w.trim_start_matches('v'))
        .find(|w| {
            w.split('.').count() >= 2
                && w.split('.').all(|part| {
                    !part.is_empty() && part.chars().next().is_some_and(|c| c.is_ascii_digit())
                })
        })
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(manager: &str, name: &str, version: &str) -> Program {
        Program {
            manager: manager.into(),
            name: name.into(),
            version: version.into(),
            present: String::new(),
            install: String::new(),
        }
    }

    fn declared_line(name: &str, version: &str, install: &str) -> Program {
        Program {
            manager: SCRIPT.into(),
            name: name.into(),
            version: version.into(),
            present: String::new(),
            install: install.into(),
        }
    }

    #[test]
    fn a_declared_program_keeps_the_line_that_installs_it() {
        let list = vec![declared_line(
            "uv",
            "0.4.9",
            "curl -LsSf https://x/uv.sh | sh",
        )];
        let back = read(&write(&list));
        assert_eq!(back, list);
        assert_eq!(
            install_command(&back[0]),
            Some(vec![
                "sh".into(),
                "-c".into(),
                "curl -LsSf https://x/uv.sh | sh".into()
            ])
        );
    }

    #[test]
    fn a_declared_program_with_no_version_still_carries_its_line() {
        let list = vec![declared_line("nvm", "", "curl -o- https://x/nvm.sh | bash")];
        assert_eq!(read(&write(&list)), list);
    }

    #[test]
    fn a_line_holding_a_tab_is_not_cut_in_half() {
        // A shell line may hold anything, tabs included. The install field is
        // the rest of the line, not the next field.
        let list = vec![declared_line("odd", "1.0", "printf 'a\tb' | cat")];
        assert_eq!(read(&write(&list))[0].install, "printf 'a\tb' | cat");
    }

    #[test]
    fn a_program_whose_command_is_not_its_name_carries_its_own_test() {
        let mut p = declared_line("node", "24.0.0", "curl -o- https://x/nvm.sh | bash");
        p.present = "nvm ls >/dev/null 2>&1".into();
        let back = read(&write(&[p.clone()]));
        assert_eq!(back, vec![p.clone()]);
        assert_eq!(back[0].presence_test(), "nvm ls >/dev/null 2>&1");
    }

    #[test]
    fn with_no_test_of_its_own_the_name_is_the_command() {
        assert_eq!(
            p("brew", "ripgrep", "").presence_test(),
            "command -v ripgrep >/dev/null 2>&1"
        );
    }

    #[test]
    fn a_reader_that_does_not_know_script_skips_it_rather_than_running_something() {
        // What an older kitbag does with a line it has never seen: nothing.
        let older = Program {
            manager: "script".into(),
            name: "uv".into(),
            version: String::new(),
            present: String::new(),
            install: String::new(),
        };
        assert_eq!(install_command(&older), None);
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
