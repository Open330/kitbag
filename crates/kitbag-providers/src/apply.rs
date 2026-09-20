//! Doing what the plan said, and nothing else.
//!
//! Two rules hold this file together.
//!
//! **A step that could not be planned is not performed.** A provider that
//! answered [`Action::Unknown`] did not know the current state, and acting
//! without knowing the current state is how a tool overwrites something it was
//! never asked to touch.
//!
//! **Nothing is destroyed to make room.** A real file where a symlink belongs
//! is moved aside first, and the report says where it went. That case - a
//! config somebody wrote by hand, sitting where the recipe wants its own link -
//! is the one that loses work.

use std::path::{Path, PathBuf};
use std::process::Command as Proc;
use std::time::{SystemTime, UNIX_EPOCH};

use kitbag_core::recipe::{Command, DefaultsKey, Link, Recipe};

use crate::{plan_command, plan_defaults, plan_link, plan_pkg, Action, PackageManager};

/// What actually happened to one resource.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Done {
    /// Already so; nothing was touched.
    Skipped,
    Created(String),
    /// Something was there. Where it went, if it was worth keeping.
    Replaced {
        what: String,
        backup: Option<PathBuf>,
    },
    /// Not attempted: the plan could not say what the current state was.
    Refused(String),
    Failed(String),
}

impl Done {
    pub fn glyph(&self) -> char {
        match self {
            Done::Skipped => '=',
            Done::Created(_) => '+',
            Done::Replaced { .. } => '~',
            Done::Refused(_) | Done::Failed(_) => '!',
        }
    }

    pub fn says(&self) -> String {
        match self {
            Done::Skipped => "already so".into(),
            Done::Created(what) => what.clone(),
            Done::Replaced { what, backup: None } => what.clone(),
            Done::Replaced {
                what,
                backup: Some(p),
            } => format!("{what} — kept the old one at {}", p.display()),
            Done::Refused(why) => format!("not done: {why}"),
            Done::Failed(e) => format!("failed: {e}"),
        }
    }

    pub fn changed(&self) -> bool {
        matches!(self, Done::Created(_) | Done::Replaced { .. })
    }

    pub fn is_problem(&self) -> bool {
        matches!(self, Done::Refused(_) | Done::Failed(_))
    }
}

/// One resource, applied.
#[derive(Debug, Clone)]
pub struct Applied {
    pub recipe: String,
    pub id: String,
    pub done: Done,
}

pub fn apply_link(link: &Link, home: &Path, repo: &Path) -> Done {
    let action = plan_link(link, home, repo);
    match &action {
        Action::None => return Done::Skipped,
        Action::Unknown(why) => return Done::Refused(why.clone()),
        _ => {}
    }

    let (from, to) = link.expanded(home, repo);
    if !from.exists() {
        return Done::Failed(format!("{} is not there to link to", from.display()));
    }
    if let Some(parent) = to.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            return Done::Failed(e.to_string());
        }
    }

    // Anything already at the target is moved aside before the link is made,
    // never deleted - a hand-written config is exactly what lives here.
    let mut backup = None;
    if let Ok(meta) = std::fs::symlink_metadata(&to) {
        if meta.file_type().is_symlink() {
            if let Err(e) = std::fs::remove_file(&to) {
                return Done::Failed(e.to_string());
            }
        } else {
            let path = backup_path(&to);
            if let Err(e) = std::fs::rename(&to, &path) {
                return Done::Failed(e.to_string());
            }
            backup = Some(path);
        }
    }

    match std::os::unix::fs::symlink(&from, &to) {
        Ok(()) => match action {
            Action::Create(what) => Done::Created(what),
            _ => Done::Replaced {
                what: "relinked".into(),
                backup,
            },
        },
        Err(e) => Done::Failed(e.to_string()),
    }
}

pub fn apply_pkg(name: &str, manager: Option<&PackageManager>) -> Done {
    match plan_pkg(name, manager) {
        Action::None => Done::Skipped,
        Action::Unknown(why) => Done::Refused(why),
        _ => {
            let Some(m) = manager else {
                return Done::Refused("no package manager".into());
            };
            let out = match m {
                PackageManager::Brew => Proc::new("brew").args(["install", name]).output(),
                PackageManager::Apt => Proc::new("sudo")
                    .args(["apt-get", "install", "-y", name])
                    .output(),
            };
            match out {
                Ok(o) if o.status.success() => {
                    Done::Created(format!("installed with {}", m.name()))
                }
                Ok(o) => Done::Failed(last_line(&String::from_utf8_lossy(&o.stderr))),
                Err(e) => Done::Failed(e.to_string()),
            }
        }
    }
}

pub fn apply_defaults(key: &DefaultsKey) -> Done {
    let action = plan_defaults(key);
    match &action {
        Action::None => return Done::Skipped,
        Action::Unknown(why) => return Done::Refused(why.clone()),
        _ => {}
    }

    let value = match &key.value {
        toml::Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    let type_flag = format!("-{}", key.kind);
    let out = Proc::new("defaults")
        .args(["write", &key.domain, &key.key, &type_flag, &value])
        .output();

    match out {
        Ok(o) if o.status.success() => match action {
            Action::Create(what) => Done::Created(what),
            Action::Replace(what) => Done::Replaced { what, backup: None },
            _ => Done::Skipped,
        },
        Ok(o) => Done::Failed(last_line(&String::from_utf8_lossy(&o.stderr))),
        Err(e) => Done::Failed(e.to_string()),
    }
}

pub fn apply_command(cmd: &Command) -> Done {
    match plan_command(cmd) {
        Action::None => Done::Skipped,
        Action::Unknown(why) => Done::Refused(why),
        _ => match Proc::new("sh").arg("-c").arg(&cmd.run).output() {
            Ok(o) if o.status.success() => Done::Created("ran".into()),
            Ok(o) => Done::Failed(last_line(&String::from_utf8_lossy(&o.stderr))),
            Err(e) => Done::Failed(e.to_string()),
        },
    }
}

/// Everything a recipe would change, changed.
///
/// Each provider re-checks the current state as it goes rather than trusting a
/// plan computed a moment ago: between the two the machine may have moved, and
/// the check is cheap next to the write.
pub fn apply_recipe(
    recipe: &Recipe,
    home: &Path,
    repo: &Path,
    manager: Option<&PackageManager>,
    with_defaults: bool,
) -> Vec<Applied> {
    let mut out = Vec::new();
    let mut push = |id: String, done: Done| {
        out.push(Applied {
            recipe: recipe.name.clone(),
            id,
            done,
        })
    };

    for name in &recipe.pkg {
        push(format!("pkg:{name}"), apply_pkg(name, manager));
    }
    for link in &recipe.link {
        push(format!("link:{}", link.to), apply_link(link, home, repo));
    }
    for key in &recipe.defaults {
        let done = if with_defaults {
            apply_defaults(key)
        } else {
            Done::Refused("defaults are a macOS thing".into())
        };
        push(format!("defaults:{}.{}", key.domain, key.key), done);
    }
    for cmd in &recipe.command {
        push(format!("run:{}", short(&cmd.run)), apply_command(cmd));
    }
    out
}

/// `<path>.backup.<utc timestamp>`, so two runs on one day do not collide and
/// the name says when without anyone having to stat it.
fn backup_path(path: &Path) -> PathBuf {
    let stamp = utc_stamp(SystemTime::now());
    let mut name = path.as_os_str().to_os_string();
    name.push(format!(".backup.{stamp}"));
    PathBuf::from(name)
}

/// `YYYYmmddHHMMSS` in UTC, without a date library: the civil-from-days
/// algorithm is short, exact, and has no timezone to be wrong about.
fn utc_stamp(t: SystemTime) -> String {
    let secs = t
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0) as i64;
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (h, mi, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);

    // Howard Hinnant's civil_from_days, with the era shifted to 0000-03-01.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };

    format!("{y:04}{m:02}{d:02}{h:02}{mi:02}{s:02}")
}

fn last_line(s: &str) -> String {
    s.lines()
        .rfind(|l| !l.trim().is_empty())
        .unwrap_or("no output")
        .trim()
        .to_string()
}

fn short(cmd: &str) -> String {
    let words: Vec<&str> = cmd.split_whitespace().take(3).collect();
    words.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::Duration;

    fn link(to: &str) -> Link {
        Link {
            from: "configs/.zshrc".into(),
            to: to.into(),
        }
    }

    fn repo_with_source() -> tempfile::TempDir {
        let repo = tempfile::tempdir().unwrap();
        fs::create_dir_all(repo.path().join("configs")).unwrap();
        fs::write(repo.path().join("configs/.zshrc"), "from the repo\n").unwrap();
        repo
    }

    #[test]
    fn a_missing_link_is_made() {
        let home = tempfile::tempdir().unwrap();
        let repo = repo_with_source();

        let done = apply_link(&link("~/.zshrc"), home.path(), repo.path());

        assert!(done.changed(), "{done:?}");
        let target = home.path().join(".zshrc");
        assert!(target.is_symlink());
        assert_eq!(fs::read_to_string(&target).unwrap(), "from the repo\n");
    }

    #[test]
    fn applying_twice_changes_nothing_the_second_time() {
        let home = tempfile::tempdir().unwrap();
        let repo = repo_with_source();

        apply_link(&link("~/.zshrc"), home.path(), repo.path());
        let again = apply_link(&link("~/.zshrc"), home.path(), repo.path());

        assert_eq!(again, Done::Skipped);
    }

    #[test]
    fn somebody_elses_config_is_moved_aside_never_deleted() {
        let home = tempfile::tempdir().unwrap();
        let repo = repo_with_source();
        let target = home.path().join(".zshrc");
        fs::write(&target, "written by hand, years ago\n").unwrap();

        let done = apply_link(&link("~/.zshrc"), home.path(), repo.path());

        let Done::Replaced {
            backup: Some(kept), ..
        } = &done
        else {
            panic!("expected the old file to be kept, got {done:?}");
        };
        assert_eq!(
            fs::read_to_string(kept).unwrap(),
            "written by hand, years ago\n",
            "the hand-written config survived"
        );
        assert!(target.is_symlink());
    }

    #[test]
    fn a_link_pointing_elsewhere_is_repointed_without_a_backup() {
        let home = tempfile::tempdir().unwrap();
        let repo = repo_with_source();
        let elsewhere = home.path().join("other");
        fs::write(&elsewhere, "x").unwrap();
        std::os::unix::fs::symlink(&elsewhere, home.path().join(".zshrc")).unwrap();

        let done = apply_link(&link("~/.zshrc"), home.path(), repo.path());

        // A symlink holds nothing of its own, so there is nothing to keep.
        assert!(
            matches!(done, Done::Replaced { backup: None, .. }),
            "{done:?}"
        );
        assert_eq!(
            fs::read_link(home.path().join(".zshrc")).unwrap(),
            repo.path().join("configs/.zshrc")
        );
    }

    #[test]
    fn a_directory_in_the_way_is_refused_rather_than_cleared() {
        let home = tempfile::tempdir().unwrap();
        let repo = repo_with_source();
        fs::create_dir(home.path().join(".zshrc")).unwrap();

        let done = apply_link(&link("~/.zshrc"), home.path(), repo.path());

        assert!(matches!(done, Done::Refused(_)), "{done:?}");
        assert!(
            home.path().join(".zshrc").is_dir(),
            "the directory is untouched"
        );
    }

    #[test]
    fn a_source_that_is_not_there_fails_before_touching_the_target() {
        let home = tempfile::tempdir().unwrap();
        let repo = tempfile::tempdir().unwrap(); // no configs/.zshrc in it
        fs::write(home.path().join(".zshrc"), "mine\n").unwrap();

        let done = apply_link(&link("~/.zshrc"), home.path(), repo.path());

        assert!(matches!(done, Done::Failed(_)), "{done:?}");
        assert_eq!(
            fs::read_to_string(home.path().join(".zshrc")).unwrap(),
            "mine\n",
            "nothing was moved for a link that could never be made"
        );
    }

    #[test]
    fn a_command_whose_check_already_passes_is_not_run_again() {
        let done = apply_command(&Command {
            run: "exit 1".into(), // would fail if it ran
            check: "true".into(),
        });
        assert_eq!(done, Done::Skipped);
    }

    #[test]
    fn a_command_that_fails_reports_what_it_said() {
        let done = apply_command(&Command {
            run: "echo 'no such thing' >&2; exit 3".into(),
            check: "false".into(),
        });
        match done {
            Done::Failed(msg) => assert!(msg.contains("no such thing"), "{msg}"),
            other => panic!("expected a failure, got {other:?}"),
        }
    }

    #[test]
    fn the_backup_name_says_when_in_utc() {
        // 2026-09-20T10:30:00Z, checked against a date library rather than
        // against my own arithmetic.
        let t = UNIX_EPOCH + Duration::from_secs(1_789_900_200);
        assert_eq!(utc_stamp(t), "20260920103000");
        // and the epoch itself, as a fencepost
        assert_eq!(utc_stamp(UNIX_EPOCH), "19700101000000");
    }
}
