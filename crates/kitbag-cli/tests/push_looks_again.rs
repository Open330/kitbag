//! Four machines pushing at the same moment, which is the ordinary case and
//! not the unlucky one.
//!
//! Two things went wrong the first time all four ran together. The store
//! refused the writes whose base another machine had moved past — correctly —
//! and kitbag reported that as a failure and stopped. And the reason it
//! printed was node complaining about a deprecated module, because that
//! arrived on the same stream first.

use std::path::Path;
use std::process::{Command, Output};

fn stub() -> &'static str {
    concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../kitbag-vault/tests/fake-bw.sh"
    )
}

fn run(home: &Path, state: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_kitbag"))
        .args(args)
        .env("HOME", home)
        .env("KITBAG_CONFIG", home.join("machine.toml"))
        .env("KITBAG_BW", stub())
        .env("KITBAG_FAKE_STATE", state)
        .env("NO_COLOR", "1")
        .output()
        .expect("kitbag runs")
}

fn push(home: &Path, state: &Path) -> String {
    let out = run(home, state, &["push", "--backend", "bw"]);
    assert!(
        out.status.success(),
        "push failed: {}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).to_string()
}

const PLAIN: &str = "scopes = [\"personal\"]\n\n[[track]]\npath = \"~/.envs/one.env\"\n";

const VOLATILE: &str =
    "scopes = [\"personal\"]\n\n[[track]]\npath = \"~/.envs/one.env\"\nvolatile = true\n";

/// A machine holding one tracked file, already agreed with the store.
fn settled() -> (tempfile::TempDir, tempfile::TempDir) {
    settled_with(PLAIN)
}

fn settled_with(config: &str) -> (tempfile::TempDir, tempfile::TempDir) {
    let home = tempfile::tempdir().expect("home");
    let state = tempfile::tempdir().expect("state");
    std::fs::create_dir_all(home.path().join(".envs")).unwrap();
    std::fs::write(
        home.path().join(".envs/one.env"),
        "# scope: personal\nA=1\n",
    )
    .unwrap();
    std::fs::write(home.path().join("machine.toml"), config).unwrap();
    push(home.path(), state.path());
    (home, state)
}

/// Move this machine on, so it has something to send.
fn edit_here(home: &Path) {
    std::fs::write(home.join(".envs/one.env"), "# scope: personal\nA=1\nB=2\n").unwrap();
}

#[test]
fn a_write_refused_because_the_store_moved_is_looked_at_again_not_given_up_on() {
    let (home, state) = settled();
    edit_here(home.path());
    // The store refuses once. Nothing else wrote, so the second look finds
    // the item is still this machine's to send.
    std::fs::write(state.path().join("stale-once"), "").unwrap();

    let out = push(home.path(), state.path());
    assert!(out.contains("sent, second time"), "{out}");
    assert!(out.contains("1 sent"), "{out}");
}

#[test]
fn a_write_refused_because_somebody_else_wrote_becomes_a_conflict_not_a_retry() {
    let (home, state) = settled();
    edit_here(home.path());
    // This time the other machine's write actually lands. Sending again would
    // write it away, and nothing here knows whose it was.
    std::fs::write(state.path().join("stale-once"), "").unwrap();
    std::fs::write(state.path().join("stale-also-writes"), "").unwrap();

    let out = push(home.path(), state.path());
    assert!(out.contains("not settled"), "{out}");
    assert!(!out.contains("sent, second time"), "{out}");
    assert!(out.contains("0 sent"), "{out}");
}

#[test]
fn what_is_reported_is_the_reason_and_not_node_complaining_about_punycode() {
    let (home, state) = settled();
    edit_here(home.path());
    // Refused every time: the second look reaches the same wall.
    std::fs::write(state.path().join("refuse-edit"), "").unwrap();

    let out = run(home.path(), state.path(), &["push", "--backend", "bw"]);
    let said = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!out.status.success(), "{said}");
    assert!(said.contains("out of date"), "{said}");
    assert!(!said.contains("punycode"), "{said}");
    assert!(!said.contains("DeprecationWarning"), "{said}");
}

#[test]
fn an_item_nobody_can_compare_is_not_worth_arguing_over() {
    // A volatile item is sent on every push, because no comparison can say it
    // was not needed. Four machines doing that to one item collide by design,
    // and there is nothing to win: whichever copy arrived is as good.
    let (home, state) = settled_with(VOLATILE);
    std::fs::write(state.path().join("stale-once"), "").unwrap();

    let out = push(home.path(), state.path());
    assert!(out.contains("another machine's copy landed first"), "{out}");
    assert!(out.contains("left to another machine's copy"), "{out}");
    assert!(!out.contains("sent, second time"), "{out}");
}
