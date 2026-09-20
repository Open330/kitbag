//! A push has to notice a marker that changed, not only bytes that did.
//!
//! Found by tagging four items `platform = ["macos"]` and pushing: "1 sent, 38
//! already there". The payloads had not moved, and the comparison was over
//! payloads, so the store kept the old answer and said nothing.

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

/// A machine holding one file, tracked by the given config body.
fn machine(config: &str) -> (tempfile::TempDir, tempfile::TempDir) {
    let home = tempfile::tempdir().expect("home");
    let state = tempfile::tempdir().expect("state");
    std::fs::create_dir_all(home.path().join(".envs")).unwrap();
    std::fs::write(
        home.path().join(".envs/one.env"),
        "# scope: personal\nA=1\n",
    )
    .unwrap();
    std::fs::write(home.path().join("machine.toml"), config).unwrap();
    (home, state)
}

const PLAIN: &str = "scopes = [\"personal\"]\n\n[[track]]\npath = \"~/.envs/one.env\"\n";

#[test]
fn a_second_push_with_nothing_changed_sends_nothing() {
    let (home, state) = machine(PLAIN);
    kitbag(home.path(), state.path(), &["push", "--backend", "bw"]);
    let out = kitbag(home.path(), state.path(), &["push", "--backend", "bw"]);
    assert!(out.contains("0 sent, 1 already there"), "{out}");
}

#[test]
fn adding_a_platform_tag_is_something_to_send() {
    let (home, state) = machine(PLAIN);
    kitbag(home.path(), state.path(), &["push", "--backend", "bw"]);

    std::fs::write(
        home.path().join("machine.toml"),
        "scopes = [\"personal\"]\n\n[[track]]\npath = \"~/.envs/one.env\"\nplatform = [\"macos\"]\n",
    )
    .unwrap();

    let out = kitbag(home.path(), state.path(), &["push", "--backend", "bw"]);
    assert!(
        out.contains("1 sent"),
        "the tag has to reach the store:\n{out}"
    );
}

#[test]
fn changing_an_owner_is_something_to_send() {
    let (home, state) = machine(PLAIN);
    kitbag(home.path(), state.path(), &["push", "--backend", "bw"]);

    std::fs::write(
        home.path().join("machine.toml"),
        "scopes = [\"personal\"]\n\n[[track]]\npath = \"~/.envs/one.env\"\nowner = \"acme\"\n",
    )
    .unwrap();

    let out = kitbag(home.path(), state.path(), &["push", "--backend", "bw"]);
    assert!(out.contains("1 sent"), "{out}");
}

fn calls(state: &Path) -> Vec<String> {
    std::fs::read_to_string(state.join("calls.log"))
        .unwrap_or_default()
        .lines()
        .map(|l| l.trim().to_string())
        .collect()
}

#[test]
fn updating_one_attachment_item_costs_five_calls() {
    // Every call to the Bitwarden client is a node process costing over a
    // second on the machine this was measured on, so the count is the runtime.
    // It was six: a folder listing that the item listing already answered, and
    // a re-read of what had just been written that nothing goes on to read.
    let (home, state) = machine(PLAIN);
    std::fs::write(
        home.path().join(".envs/one.env"),
        format!("# scope: personal\nA={}\n", "x".repeat(30_000)),
    )
    .unwrap();
    kitbag(home.path(), state.path(), &["push", "--backend", "bw"]);

    std::fs::write(state.path().join("calls.log"), "").expect("reset");
    std::fs::write(
        home.path().join(".envs/one.env"),
        format!("# scope: personal\nA={}\n", "y".repeat(30_000)),
    )
    .unwrap();
    kitbag(home.path(), state.path(), &["push", "--backend", "bw"]);

    let made = calls(state.path());
    assert_eq!(
        made,
        vec![
            "sync",
            "list items",
            "edit item",
            "create attachment",
            "delete attachment",
        ],
        "one sync, one listing, and three writes that each have to happen"
    );
}
