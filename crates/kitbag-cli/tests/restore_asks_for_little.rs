//! What a restore actually asks the vault for.
//!
//! A dry run writes nothing, and a file already identical to what the store
//! holds needs nothing fetched to establish that — the store reports the hash.
//! Getting this wrong is not visible in the output: the run is simply slow, and
//! with a client that costs over a second a call, slow enough to be unusable.
//! So it is counted.

use std::path::Path;
use std::process::Command;

fn stub() -> &'static str {
    concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../kitbag-vault/tests/fake-bw.sh"
    )
}

fn kitbag(home: &Path, state: &Path, args: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_kitbag"))
        .args(args)
        .env("HOME", home)
        .env("KITBAG_CONFIG", home.join("machine.toml"))
        .env("KITBAG_BW", stub())
        .env("KITBAG_FAKE_STATE", state)
        .env("NO_COLOR", "1")
        .output()
        .expect("kitbag runs");
    assert!(
        out.status.success(),
        "kitbag {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn calls(state: &Path, what: &str) -> usize {
    std::fs::read_to_string(state.join("calls.log"))
        .unwrap_or_default()
        .lines()
        .filter(|line| line.trim() == what)
        .count()
}

#[test]
fn a_dry_run_over_files_that_match_fetches_nothing() {
    let home = tempfile::tempdir().expect("home");
    let state = tempfile::tempdir().expect("state");
    let (home, state) = (home.path(), state.path());

    std::fs::create_dir_all(home.join(".envs")).unwrap();
    // One small item, and one large enough to be stored as an attachment —
    // which is the fetch that hurts.
    std::fs::write(home.join(".envs/small.env"), "# scope: personal\nA=1\n").unwrap();
    std::fs::write(
        home.join(".envs/big.env"),
        format!("# scope: personal\nB={}\n", "x".repeat(30_000)),
    )
    .unwrap();
    std::fs::write(
        home.join("machine.toml"),
        "scopes = [\"personal\"]\n\n[[track]]\npath = \"~/.envs/*.env\"\n",
    )
    .unwrap();

    kitbag(home, state, &["push", "--backend", "bw"]);
    std::fs::write(state.join("calls.log"), "").expect("reset the log");

    let out = kitbag(home, state, &["restore", "--backend", "bw", "--dry-run"]);

    assert!(
        out.contains("2 already here"),
        "both files match what was just pushed:\n{out}"
    );
    assert_eq!(
        calls(state, "get attachment"),
        0,
        "nothing is downloaded to discover that a file already matches"
    );
    assert_eq!(
        calls(state, "list items"),
        1,
        "and the vault is listed once"
    );
}

#[test]
fn a_file_that_differs_is_the_one_that_gets_fetched() {
    let home = tempfile::tempdir().expect("home");
    let state = tempfile::tempdir().expect("state");
    let (home, state) = (home.path(), state.path());

    std::fs::create_dir_all(home.join(".envs")).unwrap();
    std::fs::write(
        home.join(".envs/big.env"),
        format!("# scope: personal\nB={}\n", "x".repeat(30_000)),
    )
    .unwrap();
    std::fs::write(
        home.join("machine.toml"),
        "scopes = [\"personal\"]\n\n[[track]]\npath = \"~/.envs/*.env\"\n",
    )
    .unwrap();

    kitbag(home, state, &["push", "--backend", "bw"]);

    // Local edit: now it differs from the store, so it has to be read.
    std::fs::write(home.join(".envs/big.env"), "# scope: personal\nB=changed\n").unwrap();
    std::fs::write(state.join("calls.log"), "").expect("reset the log");

    let out = kitbag(home, state, &["restore", "--backend", "bw", "--dry-run"]);

    assert!(out.contains("1 to write"), "{out}");
    assert_eq!(
        calls(state, "get attachment"),
        1,
        "the one that differs is fetched, and only that one"
    );
}

#[test]
fn an_item_for_another_platform_is_not_written_here() {
    // Tagged `windows`, so whatever this test is running on, it is not for
    // here. A macOS keychain landing on a Linux server is the real case, and
    // it looks like it worked.
    let home = tempfile::tempdir().expect("home");
    let state = tempfile::tempdir().expect("state");
    let (home, state) = (home.path(), state.path());

    std::fs::create_dir_all(home.join(".envs")).unwrap();
    std::fs::write(home.join(".envs/here.env"), "# scope: personal\nA=1\n").unwrap();
    std::fs::write(home.join(".envs/elsewhere.env"), "# scope: personal\nB=2\n").unwrap();
    std::fs::write(
        home.join("machine.toml"),
        r#"scopes = ["personal"]

[[track]]
path = "~/.envs/here.env"

[[track]]
path = "~/.envs/elsewhere.env"
platform = ["windows"]
"#,
    )
    .unwrap();

    kitbag(home, state, &["push", "--backend", "bw"]);

    // Take both away, so a restore would write them if it were willing to.
    std::fs::remove_file(home.join(".envs/here.env")).unwrap();
    std::fs::remove_file(home.join(".envs/elsewhere.env")).unwrap();

    let out = kitbag(home, state, &["restore", "--backend", "bw"]);

    assert!(out.contains("for another platform"), "{out}");
    assert!(
        home.join(".envs/here.env").exists(),
        "what belongs here is written:\n{out}"
    );
    assert!(
        !home.join(".envs/elsewhere.env").exists(),
        "what does not, is not:\n{out}"
    );
}

#[test]
fn a_machine_that_tracks_nothing_still_learns_how_state_goes_back() {
    // The restore command used to live only in the restoring machine's config,
    // and that config is built from what is already installed here. A machine
    // being restored has none of it installed, so it could never be told how to
    // put an application's state back — the one case a restore exists for.
    let sender = tempfile::tempdir().expect("sender");
    let state = tempfile::tempdir().expect("state");

    std::fs::write(
        sender.path().join("machine.toml"),
        r#"scopes = ["personal"]

[[track]]
name = "app:thing"
scope = "personal"
command = { export = "echo carried", restore = "cat > \"$HOME/came-back.txt\"" }
"#,
    )
    .unwrap();
    kitbag(sender.path(), state.path(), &["push", "--backend", "bw"]);

    // A different machine: it holds nothing and tracks nothing.
    let fresh = tempfile::tempdir().expect("fresh");
    std::fs::write(
        fresh.path().join("machine.toml"),
        "scopes = [\"personal\"]\n",
    )
    .unwrap();

    let out = kitbag(fresh.path(), state.path(), &["restore", "--backend", "bw"]);

    assert!(
        out.contains("from the store"),
        "and it says where the command came from:\n{out}"
    );
    assert_eq!(
        std::fs::read_to_string(fresh.path().join("came-back.txt")).unwrap(),
        "carried\n",
        "the application's state went back through the application:\n{out}"
    );
}
