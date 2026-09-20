# Contributing

## Nobody's machine goes in this repository

kitbag exists to keep one person's machine out of a public repository. A version
of it developed by copying its author's real files in here would have argued
against its own design.

- **Fixtures are written, not collected.** Tests build a temporary `HOME` and
  put made-up files in it. Nothing reads `$HOME`, ever — not in a test, not in
  a doctest, not "just to check something".
- **Secrets in tests are assembled at runtime** (`format!("{}{}", "ghp_", …)`),
  never typed as literals. A literal would be a finding in this repository and
  would trip every scanner between here and GitHub.
- **The rule set runs over this repository too.** `crates/kitbag-cli/tests/hygiene.rs`
  walks every tracked file through `kitbag_core::lint` on each CI run: credential
  prefixes, private-key headers, absolute paths out of somebody's home directory,
  and long high-entropy strings. A line that must show what a token looks like
  marks itself `lint:allow`.
- **Findings never quote what they found.** A linter that prints the secret it
  caught has moved it into a build log.

If you want to try kitbag against your own machine, do it from a checkout you do
not push, with a scratch store (`--backend memory`), and let `kitbag lint` decide
what may be committed.

## Working on it

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
```

CI runs the same three on Linux and macOS. `DESIGN.md` is the argument; if a
change makes part of it wrong, change that part in the same commit.
