//! `lint` and `doctor` — the two commands that only ever look.
//!
//! `lint` answers "may this be committed"; `doctor` answers "is this machine
//! set up to work". Both are read-only by construction, and both report what
//! they found rather than fixing it: a tool that silently repairs a machine is
//! a tool whose owner stops knowing what their machine is.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Result;
use kitbag_core::collect::{collect, Reason};
use kitbag_core::state::orphans;
use kitbag_core::{lint as rules, Config};
use kitbag_vault::BackendKind;

use crate::run::{config_path, home};

/// What a check found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Health {
    Ok,
    Warn,
    Fail,
}

impl Health {
    fn glyph(self) -> &'static str {
        match self {
            Health::Ok => "✓",
            Health::Warn => "!",
            Health::Fail => "✗",
        }
    }
}

pub struct Check {
    pub health: Health,
    pub about: String,
    pub says: String,
}

/// Refuse the things that must not be committed.
///
/// Findings never quote what they found: a linter that prints the secret it
/// caught has moved it into a build log, where it will outlive the commit that
/// was rejected.
pub fn lint(paths: &[String], json: bool) -> Result<()> {
    let files = if paths.is_empty() {
        git_tracked()?
    } else {
        paths.iter().map(PathBuf::from).collect()
    };

    let mut findings = Vec::new();
    for path in &files {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue; // binary, or gone
        };
        for f in rules::check(&text) {
            findings.push((path.clone(), f));
        }
    }

    if json {
        let out: Vec<_> = findings
            .iter()
            .map(|(path, f)| {
                serde_json::json!({
                    "file": path.display().to_string(),
                    "line": f.line,
                    "rule": f.rule,
                    "masked": f.masked,
                    "why": f.why,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else if findings.is_empty() {
        println!("  {} files, nothing that should not be there.", files.len());
    } else {
        println!();
        for (path, f) in &findings {
            println!(
                "  {}:{} [{}] {} — {}",
                path.display(),
                f.line,
                f.rule,
                f.masked,
                f.why
            );
        }
        println!();
        println!(
            "  {} finding(s). A line that must show what a token looks like marks itself `lint:allow`.",
            findings.len()
        );
    }

    if findings.is_empty() {
        Ok(())
    } else {
        std::process::exit(1)
    }
}

/// Is this machine set up to do the job?
pub fn doctor(backend: Option<&str>, json: bool) -> Result<()> {
    let home = home();
    let cfg_path = config_path();
    let mut checks = Vec::new();

    // 1. Is there a configuration at all?
    let config = Config::load_or_default(&cfg_path)?;
    checks.push(if cfg_path.exists() {
        Check {
            health: Health::Ok,
            about: "machine config".into(),
            says: format!("{}", cfg_path.display()),
        }
    } else {
        Check {
            health: Health::Warn,
            about: "machine config".into(),
            says: format!(
                "none at {} — `kitbag discover` proposes one",
                cfg_path.display()
            ),
        }
    });

    // 2. Which scopes this machine is willing to hold.
    checks.push(Check {
        health: Health::Ok,
        about: "scopes".into(),
        says: if config.scopes.is_empty() {
            "personal only (nothing declared)".into()
        } else {
            config.scopes.join(", ")
        },
    });

    // 3. What is tracked, and what was passed over.
    let collected = collect(&config, &home);
    checks.push(Check {
        health: if collected.items.is_empty() {
            Health::Warn
        } else {
            Health::Ok
        },
        about: "tracked".into(),
        says: format!("{} item(s)", collected.items.len()),
    });

    let unmarked = collected
        .skipped
        .iter()
        .filter(|s| s.reason == Reason::Unmarked)
        .count();
    if unmarked > 0 {
        checks.push(Check {
            health: Health::Warn,
            about: "unmarked".into(),
            says: format!("{unmarked} file(s) carry no scope, so nothing sends them"),
        });
    }

    let missing = collected
        .skipped
        .iter()
        .filter(|s| s.reason == Reason::Missing)
        .count();
    if missing > 0 {
        checks.push(Check {
            health: Health::Warn,
            about: "missing".into(),
            says: format!("{missing} tracked path(s) are not on this machine"),
        });
    }

    // 4. Permissions. A secret every account on the machine can read is a
    //    secret this tool cannot make private again by moving it around.
    let loose: Vec<_> = collected
        .items
        .iter()
        .filter(|i| world_readable(&i.path))
        .map(|i| i.name.clone())
        .collect();
    if !loose.is_empty() {
        checks.push(Check {
            health: Health::Fail,
            about: "permissions".into(),
            says: format!("readable by others: {}", loose.join(", ")),
        });
    } else if !collected.items.is_empty() {
        checks.push(Check {
            health: Health::Ok,
            about: "permissions".into(),
            says: "every tracked file is owner-only".into(),
        });
    }

    // 5. The store: reachable, and does it hold anything this machine no
    //    longer sends?
    match BackendKind::from_str_or_default(backend).and_then(|k| k.open()) {
        Ok(store) => match store.list() {
            Ok(listing) => {
                checks.push(Check {
                    health: Health::Ok,
                    about: "store".into(),
                    says: format!("{} item(s)", listing.len()),
                });
                let remote = listing
                    .into_iter()
                    .map(|l| (l.name, l.payload_hash))
                    .collect();
                let left = orphans(&collected.items, &remote);
                if !left.is_empty() {
                    checks.push(Check {
                        health: Health::Warn,
                        about: "orphans".into(),
                        says: format!(
                            "{} in the store that this machine does not send: {}",
                            left.len(),
                            left.iter()
                                .map(|s| s.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        ),
                    });
                }
            }
            Err(e) => checks.push(Check {
                health: Health::Fail,
                about: "store".into(),
                says: first_line(&e.to_string()),
            }),
        },
        Err(e) => checks.push(Check {
            health: Health::Warn,
            about: "store".into(),
            says: first_line(&e.to_string()),
        }),
    }

    if json {
        let out: Vec<_> = checks
            .iter()
            .map(|c| {
                serde_json::json!({
                    "check": c.about,
                    "health": match c.health {
                        Health::Ok => "ok",
                        Health::Warn => "warn",
                        Health::Fail => "fail",
                    },
                    "says": c.says,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&out)?);
        return Ok(());
    }

    println!();
    for c in &checks {
        println!("  {} {:<14} {}", c.health.glyph(), c.about, c.says);
    }

    let bad = checks.iter().filter(|c| c.health == Health::Fail).count();
    let warn = checks.iter().filter(|c| c.health == Health::Warn).count();
    println!();
    match (bad, warn) {
        (0, 0) => println!("  Everything this knows how to check is in order."),
        (0, w) => println!("  {w} thing(s) worth a look."),
        (b, _) => println!("  {b} thing(s) are wrong."),
    }
    Ok(())
}

fn world_readable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|m| m.permissions().mode() & 0o077 != 0)
        .unwrap_or(false)
}

fn git_tracked() -> Result<Vec<PathBuf>> {
    let out = Command::new("git").args(["ls-files", "-z"]).output()?;
    if !out.status.success() {
        anyhow::bail!("not a git repository — name the files to check instead");
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .split('\0')
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .collect())
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or(s).trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_others_can_read_is_noticed() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let open = dir.path().join("open");
        let shut = dir.path().join("shut");
        std::fs::write(&open, "x").unwrap();
        std::fs::write(&shut, "x").unwrap();
        std::fs::set_permissions(&open, std::fs::Permissions::from_mode(0o644)).unwrap();
        std::fs::set_permissions(&shut, std::fs::Permissions::from_mode(0o600)).unwrap();

        assert!(world_readable(&open));
        assert!(!world_readable(&shut));
    }

    #[test]
    fn an_error_is_reported_by_its_first_line() {
        assert_eq!(first_line("boom\nand a stack trace\n"), "boom");
    }
}
