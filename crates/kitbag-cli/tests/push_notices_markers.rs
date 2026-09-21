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

#[test]
fn diff_says_which_keys_differ_and_never_what_they_hold() {
    // The case this was built for: one machine held two of these keys and the
    // store held four, and `status` could only say "changed". Finding out took
    // reading both files by hand over ssh.
    let (home, state) = machine(PLAIN);
    std::fs::write(
        home.path().join(".envs/one.env"),
        "# scope: personal\nDOCS_HOST=a\nDOCS_USER=secretuser\nDOCS_ROOT=r\nDOCS_URL=b\n",
    )
    .unwrap();
    kitbag(home.path(), state.path(), &["push", "--backend", "bw"]);

    std::fs::write(
        home.path().join(".envs/one.env"),
        "# scope: personal\nDOCS_HOST=a\nDOCS_URL=elsewhere\n",
    )
    .unwrap();

    let out = kitbag(home.path(), state.path(), &["diff", "--backend", "bw"]);
    assert!(out.contains("only there:"), "{out}");
    assert!(
        out.contains("DOCS_USER") && out.contains("DOCS_ROOT"),
        "{out}"
    );
    assert!(out.contains("differ:") && out.contains("DOCS_URL"), "{out}");
    assert!(
        !out.contains("secretuser") && !out.contains("elsewhere"),
        "a value reached the report:\n{out}"
    );
}

#[test]
fn only_resolves_one_item_and_leaves_the_rest() {
    // A glob, so both files are tracked — PLAIN names one file by hand.
    let (home, state) = machine("scopes = [\"personal\"]\n\n[[track]]\npath = \"~/.envs/*.env\"\n");
    std::fs::write(
        home.path().join(".envs/two.env"),
        "# scope: personal\nB=1\n",
    )
    .unwrap();
    kitbag(home.path(), state.path(), &["push", "--backend", "bw"]);

    std::fs::write(
        home.path().join(".envs/one.env"),
        "# scope: personal\nA=2\n",
    )
    .unwrap();
    std::fs::write(
        home.path().join(".envs/two.env"),
        "# scope: personal\nB=2\n",
    )
    .unwrap();

    let sent = kitbag(
        home.path(),
        state.path(),
        &["push", "--backend", "bw", "--only", "env:one"],
    );
    assert!(sent.contains("1 sent"), "{sent}");

    let left = kitbag(home.path(), state.path(), &["diff", "--backend", "bw"]);
    assert!(
        left.contains("env:two"),
        "the other one is untouched:\n{left}"
    );
    assert!(!left.contains("env:one"), "{left}");
}

/// A second machine on the same store, so a difference has two sides.
fn second_machine(state: &Path, body: &str) -> tempfile::TempDir {
    let home = tempfile::tempdir().expect("home");
    std::fs::create_dir_all(home.path().join(".envs")).unwrap();
    std::fs::write(home.path().join(".envs/one.env"), body).unwrap();
    std::fs::write(
        home.path().join("machine.toml"),
        "scopes = [\"personal\"]\n\n[[track]]\npath = \"~/.envs/*.env\"\n",
    )
    .unwrap();
    // Agreeing is what makes a later difference classifiable.
    kitbag(home.path(), state, &["push", "--backend", "bw"]);
    home
}

#[test]
fn one_side_moving_is_a_direction_not_a_question() {
    let (a, state) = machine("scopes = [\"personal\"]\n\n[[track]]\npath = \"~/.envs/*.env\"\n");
    kitbag(a.path(), state.path(), &["push", "--backend", "bw"]);
    let b = second_machine(state.path(), "# scope: personal\nA=1\n");

    // Only A moves.
    std::fs::write(a.path().join(".envs/one.env"), "# scope: personal\nA=2\n").unwrap();
    kitbag(a.path(), state.path(), &["push", "--backend", "bw"]);

    // B has nothing to send, and says which way it does have to go.
    let pushed = kitbag(b.path(), state.path(), &["push", "--backend", "bw"]);
    assert!(pushed.contains("0 sent"), "{pushed}");
    assert!(pushed.contains("newer in the store"), "{pushed}");

    // And taking it is not a question either.
    let took = kitbag(b.path(), state.path(), &["restore", "--backend", "bw"]);
    assert!(took.contains("1 written"), "{took}");
    assert!(!took.contains("not settled"), "{took}");
}

#[test]
fn both_sides_moving_stops_and_says_so() {
    let (a, state) = machine("scopes = [\"personal\"]\n\n[[track]]\npath = \"~/.envs/*.env\"\n");
    kitbag(a.path(), state.path(), &["push", "--backend", "bw"]);
    let b = second_machine(state.path(), "# scope: personal\nA=1\n");

    std::fs::write(
        a.path().join(".envs/one.env"),
        "# scope: personal\nA=from-a\n",
    )
    .unwrap();
    kitbag(a.path(), state.path(), &["push", "--backend", "bw"]);
    std::fs::write(
        b.path().join(".envs/one.env"),
        "# scope: personal\nA=from-b\n",
    )
    .unwrap();

    // Neither direction runs on its own.
    let pushed = kitbag(b.path(), state.path(), &["push", "--backend", "bw"]);
    assert!(pushed.contains("not settled"), "{pushed}");
    assert!(pushed.contains("both sides moved"), "{pushed}");
    assert!(
        pushed.contains("0 sent"),
        "nothing was written over:\n{pushed}"
    );

    let took = kitbag(b.path(), state.path(), &["restore", "--backend", "bw"]);
    assert!(took.contains("not settled"), "{took}");
    assert!(took.contains("0 written"), "nor the other way:\n{took}");

    // Naming it is how a person says they have decided.
    let settled = kitbag(
        b.path(),
        state.path(),
        &["push", "--backend", "bw", "--only", "env:one"],
    );
    assert!(settled.contains("1 sent"), "{settled}");

    // And once settled it is settled: no conflict remains.
    let after = kitbag(b.path(), state.path(), &["push", "--backend", "bw"]);
    assert!(!after.contains("not settled"), "{after}");
    assert!(after.contains("0 sent, 1 already there"), "{after}");
}

#[test]
fn resolve_without_a_terminal_asks_nothing_and_says_what_would() {
    // Piped, cron, a script: there is nobody to answer, so it must not wait
    // and must not choose. It lists what is unsettled and stops.
    let (a, state) = machine("scopes = [\"personal\"]\n\n[[track]]\npath = \"~/.envs/*.env\"\n");
    kitbag(a.path(), state.path(), &["push", "--backend", "bw"]);
    let b = second_machine(state.path(), "# scope: personal\nA=1\n");

    std::fs::write(
        a.path().join(".envs/one.env"),
        "# scope: personal\nA=from-a\n",
    )
    .unwrap();
    kitbag(a.path(), state.path(), &["push", "--backend", "bw"]);
    std::fs::write(
        b.path().join(".envs/one.env"),
        "# scope: personal\nA=from-b\n",
    )
    .unwrap();

    let out = kitbag(b.path(), state.path(), &["resolve", "--backend", "bw"]);
    assert!(out.contains("no terminal to ask in"), "{out}");
    assert!(out.contains("env:one"), "{out}");
    assert_eq!(
        std::fs::read_to_string(b.path().join(".envs/one.env")).unwrap(),
        "# scope: personal\nA=from-b\n",
        "nothing was decided for anybody:\n{out}"
    );
}

#[test]
fn resolve_has_nothing_to_say_when_nothing_is_in_conflict() {
    let (a, state) = machine("scopes = [\"personal\"]\n\n[[track]]\npath = \"~/.envs/*.env\"\n");
    kitbag(a.path(), state.path(), &["push", "--backend", "bw"]);
    let out = kitbag(a.path(), state.path(), &["resolve", "--backend", "bw"]);
    assert!(out.contains("Nothing to settle"), "{out}");
}

/// A machine with its own key, named after itself.
fn machine_with_key(name: &str, key: &str) -> tempfile::TempDir {
    let home = tempfile::tempdir().expect("home");
    std::fs::create_dir_all(home.path().join(".ssh")).unwrap();
    // Assembled, so this file does not carry a key-shaped literal: kitbag
    // lints its own source and is right to.
    let fence = format!("OPENSSH {} KEY", "PRIVATE");
    std::fs::write(
        home.path().join(".ssh/id_ed25519"),
        format!("-----BEGIN {fence}-----\n{key}\n-----END {fence}-----\n"),
    )
    .unwrap();
    std::fs::write(
        home.path().join("machine.toml"),
        format!(
            "machine = \"{name}\"\nscopes = [\"personal\"]\n\n[[track]]\n\
             path = \"~/.ssh/id_ed25519\"\nscope = \"personal\"\nper_machine = true\n"
        ),
    )
    .unwrap();
    home
}

#[test]
fn four_machines_can_each_keep_their_own_key_and_still_have_it_backed_up() {
    // Four machines derive `ssh:id_ed25519` from the same path and hold four
    // different keys. Refusing to exchange it kept them apart and left three
    // of them backed up nowhere — a key that exists in one place is gone with
    // the machine it is on.
    let state = tempfile::tempdir().expect("state");
    let a = machine_with_key("box-a", "AAAAkeyofa");
    let b = machine_with_key("box-b", "AAAAkeyofb");

    let sent_a = kitbag(a.path(), state.path(), &["push", "--backend", "bw"]);
    assert!(sent_a.contains("ssh:id_ed25519@box-a"), "{sent_a}");
    let sent_b = kitbag(b.path(), state.path(), &["push", "--backend", "bw"]);
    assert!(sent_b.contains("ssh:id_ed25519@box-b"), "{sent_b}");

    // Both are kept, under names that say whose they are.
    let both = kitbag(a.path(), state.path(), &["restore", "--backend", "bw"]);
    assert!(both.contains("belong to another machine"), "{both}");
    assert!(both.contains("ssh:id_ed25519@box-b"), "{both}");

    // And A still has A's key.
    let here = std::fs::read_to_string(a.path().join(".ssh/id_ed25519")).unwrap();
    assert!(here.contains("AAAAkeyofa"), "A's own key was written over");
    assert!(!here.contains("AAAAkeyofb"), "B's key landed on A");
}

#[test]
fn an_item_that_names_no_machine_is_taken_by_anyone() {
    // Almost everything is like this, and the new field must not change it.
    let (a, state) = machine("scopes = [\"personal\"]\n\n[[track]]\npath = \"~/.envs/*.env\"\n");
    kitbag(a.path(), state.path(), &["push", "--backend", "bw"]);

    let b = tempfile::tempdir().expect("home");
    std::fs::create_dir_all(b.path().join(".envs")).unwrap();
    std::fs::write(
        b.path().join("machine.toml"),
        "machine = \"somewhere-else\"\nscopes = [\"personal\"]\n\n[[track]]\npath = \"~/.envs/*.env\"\n",
    )
    .unwrap();

    let out = kitbag(b.path(), state.path(), &["restore", "--backend", "bw"]);
    assert!(out.contains("1 written"), "{out}");
    assert!(b.path().join(".envs/one.env").exists(), "{out}");
}
