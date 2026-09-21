//! Running the commands a track names.
//!
//! An export or a restore is a shell line, and some of those lines call
//! `kitbag` itself — `command = { export = "kitbag programs", restore =
//! "kitbag programs --restore" }` is the obvious way to record what is
//! installed. That line runs under whatever `PATH` the parent had, which is
//! not necessarily one that can find the binary now running: kitbag installs
//! into `~/.local/bin`, and a machine being set up for the first time has not
//! had a shell profile that mentions it yet. The restore would fail with
//! `kitbag: command not found` on precisely the machine that most needed it.
//!
//! So the directory the running binary sits in goes on the front of `PATH` for
//! any command kitbag starts. Nothing else about the environment is touched.

use std::process::Command;

/// `sh -c <command>`, with this binary's own directory reachable on `PATH`.
pub fn shell(command: &str) -> Command {
    let mut c = Command::new("sh");
    c.arg("-c").arg(command);
    if let Some(dir) = own_dir() {
        let existing = std::env::var("PATH").unwrap_or_default();
        c.env("PATH", with_front(&dir, &existing));
    }
    c
}

/// Like [`shell`], and with the directories a login shell would have added.
///
/// For the questions that are about the machine rather than about the caller.
/// What is installed here does not change because the command arrived over
/// ssh — but a non-interactive shell has none of the profile's PATH, so
/// `command -v claude` says no on a machine that has it, and a list of what is
/// installed comes out different depending on how it was asked for.
pub fn machine_shell(command: &str) -> Command {
    let mut c = Command::new("sh");
    c.arg("-c").arg(command).env("PATH", machine_path());
    c
}

/// The `PATH` those questions are asked with.
///
/// Given out on its own so a command with arguments can keep them as
/// arguments: joining a program and its arguments into a shell line to get
/// this would break the first one that contains a space.
pub fn machine_path() -> String {
    let mut path = std::env::var("PATH").unwrap_or_default();
    // Last first: each goes on the front, so the earliest named ends up first.
    for dir in usual_dirs().iter().rev() {
        path = with_front(dir, &path);
    }
    path
}

/// Where a user's own programs go, on every machine in this arrangement.
/// Deliberately a short list of conventions rather than a shell's whole
/// profile: reading somebody's rc files to answer this would run them.
fn usual_dirs() -> Vec<String> {
    let mut dirs: Vec<String> = Vec::new();
    if let Some(dir) = own_dir() {
        dirs.push(dir);
    }
    if let Some(home) = std::env::var_os("HOME") {
        let home = std::path::PathBuf::from(home);
        for rest in [".local/bin", "bin", ".cargo/bin"] {
            if let Some(p) = home.join(rest).to_str() {
                dirs.push(p.to_string());
            }
        }
    }
    dirs
}

/// Where this executable lives, if the OS will say.
fn own_dir() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    Some(dir.to_str()?.to_string())
}

/// `dir` first, then the inherited `PATH` — unless it is already there, since
/// a `PATH` that names the same directory twice is a `PATH` that grows by one
/// entry every time a command starts another one.
///
/// Takes the existing value rather than reading it, so a test can ask the
/// question without changing the answer for every other test in the binary.
fn with_front(dir: &str, existing: &str) -> String {
    if existing.split(':').any(|p| p == dir) {
        return existing.to_string();
    }
    if existing.is_empty() {
        return dir.to_string();
    }
    format!("{dir}:{existing}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_directory_goes_in_front_where_order_decides_the_answer() {
        assert_eq!(
            with_front("/opt/kit", "/usr/bin:/bin"),
            "/opt/kit:/usr/bin:/bin"
        );
    }

    #[test]
    fn a_directory_already_there_is_not_added_again() {
        assert_eq!(
            with_front("/opt/kit", "/usr/bin:/opt/kit:/bin"),
            "/usr/bin:/opt/kit:/bin"
        );
    }

    #[test]
    fn several_directories_keep_the_order_they_were_named_in() {
        let mut path = "/usr/bin".to_string();
        for dir in ["/first", "/second", "/third"].iter().rev() {
            path = with_front(dir, &path);
        }
        assert_eq!(path, "/first:/second:/third:/usr/bin");
    }

    #[test]
    fn an_empty_path_becomes_the_directory_rather_than_a_stray_colon() {
        assert_eq!(with_front("/opt/kit", ""), "/opt/kit");
    }
}
