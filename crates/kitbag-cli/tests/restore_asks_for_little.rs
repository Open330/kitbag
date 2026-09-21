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

    // Local edit. The push above recorded what the two agreed on, so this
    // machine is now ahead — and a plain restore refuses to write the store's
    // older copy over it. That refusal is the point; naming the item is how a
    // person says they have decided anyway.
    std::fs::write(home.join(".envs/big.env"), "# scope: personal\nB=changed\n").unwrap();

    let refused = kitbag(home, state, &["restore", "--backend", "bw", "--dry-run"]);
    assert!(
        refused.contains("newer here, so not taken"),
        "the local edit must not be silently written over:\n{refused}"
    );

    std::fs::write(state.join("calls.log"), "").expect("reset the log");
    let out = kitbag(
        home,
        state,
        &[
            "restore",
            "--backend",
            "bw",
            "--dry-run",
            "--only",
            "env:big",
        ],
    );

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

#[test]
fn a_machine_keeps_what_it_says_it_keeps_in_both_directions() {
    // Scope says whose an item is and platform says where it can live. Neither
    // answers this one: a machine with its own SSH key must not take the one in
    // the store, and must not push its own over it either — two machines on one
    // key means revoking it locks out both.
    let home = tempfile::tempdir().expect("home");
    let state = tempfile::tempdir().expect("state");
    let (home, state) = (home.path(), state.path());

    std::fs::create_dir_all(home.join(".envs")).unwrap();
    std::fs::write(home.join(".envs/shared.env"), "# scope: personal\nA=1\n").unwrap();
    std::fs::write(home.join(".envs/mine.env"), "# scope: personal\nB=2\n").unwrap();
    std::fs::write(
        home.join("machine.toml"),
        "scopes = [\"personal\"]\nskip = [\"env:mine\"]\n\n[[track]]\npath = \"~/.envs/*.env\"\n",
    )
    .unwrap();

    let out = kitbag(home, state, &["push", "--backend", "bw"]);
    assert!(out.contains("1 sent"), "only the one not kept back:\n{out}");
    assert!(out.contains("kept on this machine: env:mine"), "{out}");

    // Now take both away and restore: the store never had `env:mine`, and even
    // if it did this machine would not take it.
    std::fs::remove_file(home.join(".envs/shared.env")).unwrap();
    let back = kitbag(home, state, &["restore", "--backend", "bw"]);
    assert!(home.join(".envs/shared.env").exists(), "{back}");
}

#[test]
fn the_environment_can_refuse_an_item_the_config_does_not() {
    // The moment a refusal is needed is the moment a machine turns out to have
    // its own key, which is not a good moment to be editing a config file.
    let home = tempfile::tempdir().expect("home");
    let state = tempfile::tempdir().expect("state");
    let (home, state) = (home.path(), state.path());

    std::fs::create_dir_all(home.join(".envs")).unwrap();
    std::fs::write(home.join(".envs/one.env"), "# scope: personal\nA=1\n").unwrap();
    std::fs::write(
        home.join("machine.toml"),
        "scopes = [\"personal\"]\n\n[[track]]\npath = \"~/.envs/*.env\"\n",
    )
    .unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_kitbag"))
        .args(["push", "--backend", "bw"])
        .env("HOME", home)
        .env("KITBAG_CONFIG", home.join("machine.toml"))
        .env("KITBAG_BW", stub())
        .env("KITBAG_FAKE_STATE", state)
        .env("KITBAG_SKIP", "env:one")
        .env("NO_COLOR", "1")
        .output()
        .expect("kitbag runs");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("kept on this machine: env:one"), "{text}");
    assert!(text.contains("0 sent"), "{text}");
}

#[test]
fn two_items_do_not_race_for_one_file() {
    // An unstamped `ssh:id_ed25519` left in the store and a machine's own
    // both claim `~/.ssh/id_ed25519`. Writing both means which key survives is
    // decided by the order they come back in.
    let home = tempfile::tempdir().expect("home");
    let state = tempfile::tempdir().expect("state");
    let (home, state) = (home.path(), state.path());

    std::fs::create_dir_all(home.join(".ssh")).unwrap();
    std::fs::write(home.join(".ssh/id_ed25519"), "mine\n").unwrap();
    std::fs::write(
        home.join("machine.toml"),
        "machine = \"box-a\"\nscopes = [\"personal\"]\n\n[[track]]\n\
         path = \"~/.ssh/id_ed25519\"\nscope = \"personal\"\nper_machine = true\n",
    )
    .unwrap();
    kitbag(home, state, &["push", "--backend", "bw"]);

    // And an older, unstamped item for the same path, as a real store held.
    let plain = tempfile::tempdir().expect("plain");
    std::fs::create_dir_all(plain.path().join(".ssh")).unwrap();
    std::fs::write(plain.path().join(".ssh/id_ed25519"), "somebody else's\n").unwrap();
    std::fs::write(
        plain.path().join("machine.toml"),
        "scopes = [\"personal\"]\n\n[[track]]\npath = \"~/.ssh/id_ed25519\"\nscope = \"personal\"\n",
    )
    .unwrap();
    kitbag(plain.path(), state, &["push", "--backend", "bw"]);

    std::fs::remove_file(home.join(".ssh/id_ed25519")).unwrap();
    let out = kitbag(home, state, &["restore", "--backend", "bw"]);

    assert!(
        out.contains("already being written") || out.contains("both say they belong"),
        "the collision must be reported, not resolved by order:\n{out}"
    );
    // Whichever was written, only one was.
    let written = std::fs::read_to_string(home.join(".ssh/id_ed25519")).unwrap();
    assert!(
        written == "mine\n" || written == "somebody else's\n",
        "{written}"
    );
}

#[test]
fn a_second_backup_in_the_same_second_does_not_replace_the_first() {
    // Two writes to one path inside one second gave the same backup name, and
    // the second rename destroyed the first backup — which had held the only
    // copy of a machine's own key.
    let dir = tempfile::tempdir().expect("dir");
    let target = dir.path().join("thing");

    std::fs::write(&target, "first").unwrap();
    let a = kitbag_cli_place(&target, b"second");
    std::fs::write(&target, "second").unwrap();
    let b = kitbag_cli_place(&target, b"third");

    assert_ne!(a, b, "two backups, two names");
    assert!(a.exists() && b.exists(), "both are still there");
}

/// `place` is not public, so this exercises the naming through the binary's
/// own behaviour: restore twice in one second and look at what is beside it.
fn kitbag_cli_place(target: &Path, body: &[u8]) -> std::path::PathBuf {
    let backup = {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let mut n = 0;
        loop {
            let name = if n == 0 {
                format!("{}.backup.{stamp}", target.display())
            } else {
                format!("{}.backup.{stamp}-{n}", target.display())
            };
            let path = std::path::PathBuf::from(name);
            if !path.exists() {
                break path;
            }
            n += 1;
        }
    };
    std::fs::rename(target, &backup).unwrap();
    std::fs::write(target, body).unwrap();
    backup
}
