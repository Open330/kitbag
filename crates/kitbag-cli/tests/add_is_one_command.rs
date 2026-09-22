//! Saying "keep this" should be a command, not an edit.
//!
//! The config file exists because a machine has to remember the answer, not
//! because a person should have to type it in that shape.

use std::path::Path;
use std::process::Command;

fn kitbag(home: &Path, args: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_kitbag"))
        .args(args)
        .env("HOME", home)
        .env("KITBAG_CONFIG", home.join("machine.toml"))
        .env("KITBAG_CATALOGUE", home.join("catalogue.toml"))
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

/// A directory holding two scripts, a readme and something a package manager
/// left behind — which is what a `bin` directory actually looks like.
fn home_with_a_mixed_directory() -> tempfile::TempDir {
    let home = tempfile::tempdir().expect("a home");
    let dir = home.path().join("work/deploy");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("ship"), "#!/bin/sh\n# scope: work\necho ship\n").unwrap();
    std::fs::write(
        dir.join("rollback"),
        "#!/bin/sh\n# scope: work\necho back\n",
    )
    .unwrap();
    std::fs::write(dir.join("README"), "how to deploy\n").unwrap();
    std::fs::write(dir.join("helper"), b"\x7fELF\x02\x01\x01\0\0\0\0\0").unwrap();
    home
}

#[test]
fn a_directory_becomes_the_pattern_that_covers_what_is_not_in_it_yet() {
    let home = home_with_a_mixed_directory();
    let out = kitbag(home.path(), &["add", "~/work/deploy", "--scope", "work"]);
    assert!(out.contains("~/work/deploy/*"), "{out}");
    let config = std::fs::read_to_string(home.path().join("machine.toml")).unwrap();
    assert!(config.contains("path = \"~/work/deploy/*\""), "{config}");
}

#[test]
fn a_directory_of_scripts_and_binaries_takes_the_scripts_and_says_so() {
    // Storing a binary built for one architecture is the thing the `programs`
    // list exists to avoid, and a guess made silently is a guess nobody can
    // correct.
    let home = home_with_a_mixed_directory();
    let out = kitbag(home.path(), &["add", "~/work/deploy", "--scope", "work"]);
    assert!(out.contains("scripts only"), "{out}");
    assert!(out.contains("2 of 4"), "{out}");
    let config = std::fs::read_to_string(home.path().join("machine.toml")).unwrap();
    assert!(config.contains("only = \"scripts\""), "{config}");

    let status = kitbag(home.path(), &["status", "--scope", "all"]);
    assert!(status.contains("work-deploy-ship"), "{status}");
    assert!(!status.contains("README"), "{status}");
    assert!(!status.contains("helper"), "{status}");
}

#[test]
fn asking_for_everything_gets_everything() {
    let home = home_with_a_mixed_directory();
    kitbag(
        home.path(),
        &["add", "~/work/deploy", "--scope", "work", "--all"],
    );
    let config = std::fs::read_to_string(home.path().join("machine.toml")).unwrap();
    assert!(!config.contains("only"), "{config}");
}

#[test]
fn a_file_carrying_its_own_marker_gets_no_second_answer() {
    // A marker travels with the file. A scope written in the config as well
    // is a second answer to the same question, and the two can disagree.
    let home = tempfile::tempdir().expect("a home");
    std::fs::create_dir_all(home.path().join("notes")).unwrap();
    std::fs::write(home.path().join("notes/j.md"), "# scope: personal\nhi\n").unwrap();

    let out = kitbag(home.path(), &["add", "~/notes/j.md"]);
    assert!(out.contains("own marker"), "{out}");
    let config = std::fs::read_to_string(home.path().join("machine.toml")).unwrap();
    assert!(!config.contains("scope ="), "{config}");
}

#[test]
fn a_file_with_no_marker_and_no_scope_says_it_will_be_skipped() {
    // Rather than defaulting to a scope, which is a guess about who owns a
    // credential — the one guess this tool must not make quietly.
    let home = tempfile::tempdir().expect("a home");
    std::fs::create_dir_all(home.path().join("notes")).unwrap();
    std::fs::write(home.path().join("notes/j.md"), "no marker here\n").unwrap();
    let out = kitbag(home.path(), &["add", "~/notes/j.md"]);
    assert!(out.contains("will be skipped"), "{out}");
}

#[test]
fn adding_the_same_thing_twice_adds_it_once() {
    let home = home_with_a_mixed_directory();
    kitbag(home.path(), &["add", "~/work/deploy", "--scope", "work"]);
    let again = kitbag(home.path(), &["add", "~/work/deploy", "--scope", "work"]);
    assert!(again.contains("already tracked"), "{again}");
    let config = std::fs::read_to_string(home.path().join("machine.toml")).unwrap();
    assert_eq!(config.matches("[[track]]").count(), 1, "{config}");
}

#[test]
fn everywhere_writes_a_rule_as_well_and_relative_to_the_home() {
    // A catalogue entry is a rule for every machine, and no two of them agree
    // on what one person's home directory is called.
    let home = home_with_a_mixed_directory();
    let out = kitbag(
        home.path(),
        &[
            "add",
            "~/work/deploy",
            "--scope",
            "work",
            "--everywhere",
            "--why",
            "deploy scripts",
        ],
    );
    assert!(out.contains("every machine"), "{out}");
    let rules = std::fs::read_to_string(home.path().join("catalogue.toml")).unwrap();
    assert!(rules.contains("path = \"work/deploy/*\""), "{rules}");
    assert!(rules.contains("why = \"deploy scripts\""), "{rules}");
    assert!(rules.contains("only = \"scripts\""), "{rules}");
    assert!(!rules.contains('~'), "{rules}");

    // And the catalogue reads back what was written.
    let seen = kitbag(home.path(), &["catalogue"]);
    assert!(seen.contains("deploy scripts"), "{seen}");
}

#[test]
fn a_scope_this_version_does_not_know_is_refused_before_anything_is_written() {
    let home = home_with_a_mixed_directory();
    let out = Command::new(env!("CARGO_BIN_EXE_kitbag"))
        .args(["add", "~/work/deploy", "--scope", "nonsense"])
        .env("HOME", home.path())
        .env("KITBAG_CONFIG", home.path().join("machine.toml"))
        .env("NO_COLOR", "1")
        .output()
        .expect("kitbag runs");
    assert!(!out.status.success());
    assert!(!home.path().join("machine.toml").exists());
}

#[test]
fn what_was_added_can_be_read_back_as_it_was_written() {
    // `status` answers a different question: it shows the items a track came
    // to. Somebody who typed one line and got back two file names has no way
    // to see the line they wrote.
    let home = home_with_a_mixed_directory();
    kitbag(home.path(), &["add", "~/work/deploy", "--scope", "work"]);

    let listed = kitbag(home.path(), &["tracked"]);
    assert!(listed.contains("~/work/deploy/*"), "{listed}");
    assert!(listed.contains("work"), "{listed}");
    assert!(listed.contains("1 track"), "{listed}");
}

#[test]
fn a_filter_that_left_something_out_says_how_much() {
    // A filter nobody can see is one nobody can correct. This is the only
    // place its effect is visible: `status` shows what came through, and
    // silence about the rest reads as "there was nothing else".
    let home = home_with_a_mixed_directory();
    kitbag(home.path(), &["add", "~/work/deploy", "--scope", "work"]);
    let listed = kitbag(home.path(), &["tracked"]);
    assert!(listed.contains("scripts only"), "{listed}");
    assert!(listed.contains("filtered out"), "{listed}");
}

#[test]
fn a_track_whose_file_is_not_here_says_so_rather_than_looking_fine() {
    // Believing a path is kept when nothing is there is the failure this
    // whole tool exists to avoid.
    let home = tempfile::tempdir().expect("a home");
    std::fs::write(
        home.path().join("machine.toml"),
        "scopes = [\"personal\"]\n\n[[track]]\npath = \"~/.npmrc\"\nscope = \"personal\"\n",
    )
    .unwrap();
    let listed = kitbag(home.path(), &["tracked"]);
    assert!(listed.contains("nothing here"), "{listed}");
}

#[test]
fn a_machine_that_keeps_nothing_says_where_to_start() {
    let home = tempfile::tempdir().expect("a home");
    let listed = kitbag(home.path(), &["tracked"]);
    assert!(listed.contains("Nothing tracked yet"), "{listed}");
    assert!(listed.contains("kitbag add"), "{listed}");
}

#[test]
fn status_counts_what_a_filter_left_out() {
    // Where people already look. A filter whose effect is invisible is one
    // nobody can correct, and the files it drops look exactly like files that
    // were never there.
    let home = home_with_a_mixed_directory();
    kitbag(home.path(), &["add", "~/work/deploy", "--scope", "work"]);
    let out = kitbag(home.path(), &["status", "--scope", "all"]);
    assert!(out.contains("left out by a track's filter"), "{out}");
    assert!(out.contains("2 file(s)"), "{out}");
}

#[test]
fn status_already_names_a_tracked_path_with_nothing_behind_it() {
    // Guards the claim `tracked` was once wrongly credited with: this is
    // status's job and status was already doing it.
    let home = tempfile::tempdir().expect("a home");
    std::fs::write(
        home.path().join("machine.toml"),
        "scopes = [\"personal\"]\n\n[[track]]\npath = \"~/.npmrc\"\nscope = \"personal\"\n",
    )
    .unwrap();
    let out = kitbag(home.path(), &["status"]);
    assert!(out.contains("not on this machine"), "{out}");
}
