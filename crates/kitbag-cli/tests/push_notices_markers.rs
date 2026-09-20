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
    assert!(out.contains("1 sent"), "the tag has to reach the store:\n{out}");
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
