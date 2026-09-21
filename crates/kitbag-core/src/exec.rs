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
    fn an_empty_path_becomes_the_directory_rather_than_a_stray_colon() {
        assert_eq!(with_front("/opt/kit", ""), "/opt/kit");
    }
}
