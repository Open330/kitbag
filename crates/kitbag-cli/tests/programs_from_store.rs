//! Reading the list a machine wrote, from another machine.
//!
//! `kitbag programs` answers for the machine it runs on, which is the easy
//! half. The question that matters when a machine is gone — or new — is what
//! the *other* one had.

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

fn ok(home: &Path, state: &Path, args: &[&str]) -> String {
    let out = run(home, state, args);
    assert!(
        out.status.success(),
        "kitbag {args:?} failed: {}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).to_string()
}

/// A machine whose list is a fixed string, so this tests the store and not
/// whatever happens to be installed on the machine running the tests.
fn machine_named(name: &str) -> (tempfile::TempDir, tempfile::TempDir) {
    let home = tempfile::tempdir().expect("home");
    let state = tempfile::tempdir().expect("state");
    std::fs::write(
        home.path().join("machine.toml"),
        format!(
            "scopes = [\"personal\"]\nmachine = \"{name}\"\n\n\
             [[track]]\nname = \"programs\"\nscope = \"personal\"\nper_machine = true\n\
             command = {{ export = \"printf 'kitbag/programs 1\\\\nbrew\\\\tripgrep\\\\nscript\\\\tnosuchtool-kb\\\\t0.4.9\\\\t\\\\tcurl -LsSf https://x/uv.sh | sh\\\\n'\", restore = \"true\" }}\n"
        ),
    )
    .unwrap();
    (home, state)
}

#[test]
fn a_machine_can_read_the_list_another_one_wrote() {
    let (one, state) = machine_named("boxone");
    ok(one.path(), state.path(), &["push", "--backend", "bw"]);

    // A different machine, same store.
    let (two, _) = machine_named("boxtwo");
    let seen = ok(
        two.path(),
        state.path(),
        &["programs", "--backend", "bw", "--from", "boxone"],
    );
    assert!(seen.contains("kitbag/programs 1"), "{seen}");
    assert!(seen.contains("brew\tripgrep"), "{seen}");
    // The install line survives the round trip through the store, tabs and all.
    assert!(seen.contains("curl -LsSf https://x/uv.sh | sh"), "{seen}");
}

#[test]
fn which_machines_have_written_a_list_is_a_question_with_an_answer() {
    let (one, state) = machine_named("boxone");
    ok(one.path(), state.path(), &["push", "--backend", "bw"]);
    let (two, _) = machine_named("boxtwo");
    ok(two.path(), state.path(), &["push", "--backend", "bw"]);

    let seen = ok(
        one.path(),
        state.path(),
        &["programs", "--backend", "bw", "--list"],
    );
    assert!(seen.contains("boxone"), "{seen}");
    assert!(seen.contains("boxtwo"), "{seen}");
    assert!(seen.contains("2 list(s)"), "{seen}");
}

#[test]
fn asking_for_a_machine_that_never_wrote_one_says_so_and_says_where_to_look() {
    let (one, state) = machine_named("boxone");
    ok(one.path(), state.path(), &["push", "--backend", "bw"]);

    let out = run(
        one.path(),
        state.path(),
        &["programs", "--backend", "bw", "--from", "nosuchbox"],
    );
    let said = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(!out.status.success(), "{said}");
    assert!(said.contains("programs@nosuchbox"), "{said}");
    assert!(said.contains("--list"), "{said}");
}

#[test]
fn installing_from_another_machines_list_says_what_it_would_run() {
    let (one, state) = machine_named("boxone");
    ok(one.path(), state.path(), &["push", "--backend", "bw"]);
    let (two, _) = machine_named("boxtwo");

    let seen = ok(
        two.path(),
        state.path(),
        &[
            "programs",
            "--backend",
            "bw",
            "--from",
            "boxone",
            "--restore",
            "--dry-run",
        ],
    );
    // A line out of the store is named before it would run.
    assert!(seen.contains("(from the list)"), "{seen}");
    assert!(seen.contains("Nothing was run"), "{seen}");
}
