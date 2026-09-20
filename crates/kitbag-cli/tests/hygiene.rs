//! kitbag lints its own repository.
//!
//! This tool exists to keep one person's machine out of a public repository. A
//! version of it that leaked its author's machine while being written would
//! have argued against its own design, so the rule set runs over every tracked
//! file here, on every CI run.

use std::path::Path;
use std::process::Command;

#[test]
fn no_tracked_file_carries_a_secret_or_a_personal_path() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root")
        .to_path_buf();

    let out = Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["ls-files", "-z"])
        .output()
        .expect("git ls-files");
    assert!(out.status.success(), "git ls-files failed");

    let mut problems = Vec::new();
    for rel in String::from_utf8_lossy(&out.stdout).split('\0') {
        if rel.is_empty() {
            continue;
        }
        let path = root.join(rel);
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue; // binary, or gone: nothing to read
        };
        for f in kitbag_core::lint::check(&content) {
            problems.push(format!(
                "{rel}:{} [{}] {} — {}",
                f.line, f.rule, f.masked, f.why
            ));
        }
    }

    assert!(
        problems.is_empty(),
        "the repository carries things it should not:\n{}",
        problems.join("\n")
    );
}
