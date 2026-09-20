# kitbag — design

> Everything this machine holds that is yours: what it is, whose it is, and how
> it gets onto the next machine.

Status: draft for review. Nothing is built yet.

## 1. Why

`jiunbae/settings` already does this job in bash, and it works. The parts that
broke, though, all broke for the same family of reasons. From one week of use:

| What happened | Root cause |
| --- | --- |
| The list of skipped files printed empty | a variable filled inside `$( )` dies with the subshell |
| A whole display column shifted by one | bash counts tab as IFS whitespace, so empty fields collapse |
| A push died mid-way, leaving the vault half-written and its inventory stale | `set -e` plus one network error |
| Remote key revocation needed hand-verified quoting | ssh inside ssh inside awk |
| A key rotation locked the machine out of the step that completes the rotation | no way to express "these two operations are one" |

None of these are bash being used badly. They are bash not having values,
errors, or types. The logic that needs those is exactly the logic that has
grown: state comparison, hashing, partial failure, remote fan-out, and a
manifest that must stay consistent with what is actually stored.

The second reason is that the repository has quietly become two things: a
personal dotfiles installer, and a general answer to "move my machine". Only the
first belongs in a personal repo.

## 2. What it is

A CLI that keeps a machine's **personal state** — packages, configuration,
system settings, credentials and app data — in a form that can be inspected,
diffed, and put back on another machine.

It is **not** a configuration manager for servers (no inventory, no SSH fleet,
no roles), **not** a dotfiles symlinker (that is one provider among many), and
**not** a secret manager (it borrows one).

The unifying idea: *packages, dotfiles, macOS defaults, and secrets are the same
kind of thing* — a desired state, a source it comes from, a way to apply it, and
an owner. Once they share a model, `plan`/`apply`/`status` work across all of
them, and so does the one concept the bash version proved was missing:

## 3. Scope is the organising idea

Every resource belongs to someone:

| scope | meaning |
| --- | --- |
| `personal` | yours |
| `work` | an employer's or client's |
| `shared` | an account someone else owns that you were given |
| `mixed` | one artifact holding several — it must say which (`spans`) |
| `local` | this machine only, never leaves it |

A machine declares which scopes it takes. A personal laptop never restores work
credentials; a work machine does not install your personal toys. The same filter
decides what a push sends, what a restore writes, and what a report shows.

This is the part with no equivalent elsewhere. chezmoi manages dotfiles;
1Password CLI manages secrets; neither can answer "what on this machine belongs
to my employer, and what happens to it when I leave".

## 4. Command surface

```
kitbag status                 what this machine has, grouped by scope, marked against the remote
kitbag plan                   what apply would change
kitbag apply [--only pkg]     make it so
kitbag discover               scan for personal state that is not tracked yet, and propose it
kitbag track <path> --scope work [--owner acme]
kitbag push [--scope ...]     send tracked state to the vault
kitbag restore [--scope ...]  write it back here
kitbag doctor                 permissions, reachability, unscoped files, orphans in the vault
kitbag diff <item>            what changed — names and shapes, never values
kitbag trust                  the machines that may log in here (folds in ssh-trust.sh)
```

Defaults that matter: `plan` is implied unless `apply`/`push`/`restore` is asked
for, output is a tree grouped by scope, `--json` exists for every command, and
no command ever prints a secret's value.

```
  work · acme  (14)
  ├── ~ env:github               GITHUB_GITEA_TOKEN GITHUB_DEPLOY_TOKEN …
  ├── = env:jenkins              JENKINS_API_KEY JENKINS_USER
  └── + file:aws-vault-keychain  binary, 25788 bytes → ~/Library/Keychains/…

  + 3 new   ~ 1 changed   = 35 unchanged   ? 1 built on push
```

## 5. Model

```rust
struct Resource {
    id: ResourceId,          // "env:github", "pkg:ripgrep", "defaults:dock.tilesize"
    scope: Scope,            // personal | work | shared | mixed{spans} | local
    owner: Option<String>,   // free text: "acme", "a friend"
    source: Source,
    sink: Sink,
}

enum Source {
    Path(PathBuf),                       // a file, tracked as-is
    Glob(String),                        // many files, each carrying its own marker
    Command { export: String },          // `aas export --all`
    Literal(String),                     // from the recipe itself
    Remote(ItemRef),                     // from the vault
}

enum Sink {
    File { dest: PathBuf, mode: u32 },   // write, backing up what was there
    Merge { dest: PathBuf, key: MergeKey },  // add missing lines, never remove
    Command(String),                     // pipe into `aas import -`
    Package(PackageSpec),                // brew, apt, cask, mas, cargo, npm
    Defaults(DefaultsSpec),              // macOS domain/key/type
    Symlink { target: PathBuf },
}

enum State { New, Changed, Unchanged, Unknown }   // Unknown: only building it would tell
```

Two rules the bash version arrived at the hard way, kept as invariants:

- **A marker travels with the file.** A `# scope:` header inside a file wins
  over any table that names it, because the table does not get copied along.
- **Merging never deletes.** A host may hold keys, lines or accounts this tool
  has never seen. Only an explicit `revoke`/`prune` removes anything.

## 6. Configuration

Three layers, deliberately separated by who may read them:

```
kitbag.toml                       in the repo, public: recipes, no names of employers
~/.config/kitbag/machine.toml     this machine: which scopes, which overrides
vault item "inventory"            private: what exists, where it goes, hashes
```

```toml
# kitbag.toml — public
[[recipe]]
name = "shell"
pkg = ["zsh", "zsh-autosuggestions", "fzf"]
link = [{ from = "configs/.zshrc", to = "~/.zshrc" }]

[[recipe]]
name = "macos"
platform = "macos"
defaults = [
  { domain = "com.apple.dock", key = "tilesize", type = "int", value = 42 },
]

[[track]]
path  = "~/.envs/*.env"
scope = "auto"        # read the file's own marker; refuse if absent

[[track]]
name    = "aas"
command = { export = "aas export --all", restore = "aas import -" }
scope   = "mixed"
spans   = ["personal", "work"]
```

```toml
# ~/.config/kitbag/machine.toml — private to the machine
name   = "june-mba"
scopes = ["personal", "work"]

[[track]]
path  = "~/Library/Keychains/aws-vault.keychain-db"
scope = "work"
owner = "acme"
```

## 7. The split: a public repo, a private inventory, a secret store

The rule that makes a dotfiles repo publishable is not "keep secrets out" —
everyone knows that. It is **keep out the list of what exists**. A public repo
that reads `~/.envs/kibana.env → work` has told the world where the author
works, what they run, and what to phish. This tool enforces the split rather
than trusting the author to remember it.

```
  repo (public)            recipes, providers, scope names. No employer, no
                           service, no path that only one person would have.
  inventory (private)      what exists, where it goes, which scope, hashes.
                           Lives in the secret store, never on disk unencrypted.
  secret store             the values themselves: a vault, or an age-encrypted
                           file that can sit in a private repo.
```

Three consequences, all enforced by the tool:

1. **Collection is by pattern, never by name.** The repo says
   `~/.envs/*.env`; it never says which ones exist. Anyone can fork it and
   their own files are picked up.
2. **Scope lives in the file, not in the repo.** `# scope: work` is a marker
   the file carries. A table in the repo mapping names to employers would be
   the leak itself.
3. **`kitbag lint` fails a commit that breaks this.** It scans the working tree
   for high-entropy strings, for known credential filenames, and for
   inventory-shaped data (item names with scopes attached) in tracked files. It
   is a pre-commit hook and a CI job, so the split survives a hurried evening.

`kitbag init` sets all three up: a repo with recipes and a lint hook, a machine
profile, and a store of the chosen backend.

## 8. Discovery

The question the bash version could not answer: *what is on this machine that I
have not thought about?* `kitbag discover` walks a built-in catalogue and a set
of heuristics, and proposes entries.

Catalogue (excerpt): `~/.aws/{config,credentials}`, `~/.kube/*.yaml`,
`~/.docker/config.json`, `~/.npmrc`, `~/.pypirc`, `~/.netrc`, `~/.gnupg/`,
`~/.ssh/id_*`, `~/.config/gh/hosts.yml`, `~/.config/rclone/rclone.conf`,
`~/.terraformrc`, `~/.cargo/credentials.toml`, `~/Library/Keychains/*.keychain-db`,
`~/.claude/.credentials.json`, `~/.codex/auth.json`, plus an app table for
things that keep state in `Application Support`.

Heuristics: files under `$HOME` with mode 600 that parse as `key = value` and
carry a high-entropy value; dotfiles modified recently that git does not track.

Each finding comes with a proposed scope and the reason for it, and is accepted
or dismissed interactively. Dismissals are remembered, so the second run is
quiet.

## 9. Architecture

```
crates/
  kitbag-core      model, scope, plan/apply engine, hashing, diff
  kitbag-providers pkg(brew|apt|cask|mas|cargo|npm) file defaults launchd command
  kitbag-vault     Backend trait; bitwarden-cli impl first, native later
  kitbag-catalog   discovery table + heuristics
  kitbag-cli       clap, tree/json output, TUI for discover and status
```

**Every backend is a first-class citizen, because the envelope does the work.**
Stores disagree about everything — Bitwarden has notes, fields and attachments;
1Password has typed fields and files; `pass` has a tree of GPG-encrypted text
files and no metadata at all. So kitbag stores its own envelope and asks a
backend only to keep bytes under a name:

```text
kitbag/1
scope: work
owner: acme
encoding: utf8
sha256: 1f0e3d…

export TOKEN=…
```

A store with native fields may mirror the header into them so its own UI shows
the scope; the envelope stays authoritative. Binary payloads are base64 inside
the envelope, because "text only" is the common case, not the exception. The
practical result: **adding a backend is a `list`/`get`/`put` adapter and nothing
else** — no scope logic, no schema, no migration.

There is deliberately no `delete` in the trait. A store holds things this
machine knows nothing about, and a tool that removes what it does not recognise
eventually removes something that mattered.

**Bitwarden and 1Password ship together in v0.1**, since the two cover most
people; `pass` follows for the GPG crowd and `age` for people with no vault at
all. The in-memory backend used by tests has *no* capabilities, which makes it
the floor: anything that works there works everywhere.

Historical note: the first implementation was a wrapper around `bw`. Implementing Bitwarden's
crypto means writing cipher code against a vault holding everything; the
performance argument for it mostly evaporated once unchanged items stopped being
rewritten (a quiet push is now two calls plus the changes). The trait keeps the
door open.

```rust
trait Backend {
    fn list(&self) -> Result<Vec<RemoteItem>>;        // name, fields, hash — not values
    fn get(&self, id: &ItemId) -> Result<Secret>;     // zeroized on drop
    fn put(&self, item: &Item) -> Result<()>;         // upsert, never delete
    fn attach(&self, id: &ItemId, blob: &Path) -> Result<()>;
}
```

Backends, in the order they are worth writing:

| Backend | Why | Cost |
| --- | --- | --- |
| **`bw`** (Bitwarden / Vaultwarden) | free tier, self-hostable, already in use here | wrapper, done first |
| **`age`** (encrypted file) | **no server at all** — the encrypted blob lives in a private repo, S3, iCloud, a USB stick. The natural partner to a public settings repo, and the only backend that works offline and outlives any vendor | native crate (`age`), no external binary |
| **`op`** (1Password) | the best developer CLI in the category — `op run`, a native SSH agent, structured item refs (`op://vault/item/field`). Paid, no free tier | wrapper |
| `pass` / `gopass` | the Unix answer: GPG plus a git repo. Already understood by anyone who has it | wrapper |
| `keychain` (macOS) | not a sync backend — a local cache so a restore does not re-prompt | `security` wrapper |

`age` matters more than its position suggests: it is what makes the tool usable
by someone with no vault at all. `kitbag init --backend age` generates a key,
writes the encrypted store beside the repo, and the whole design works from
there.

**The engine is plan/apply.** Every provider answers "what is the current state
of this resource" and "how do I reach the desired one". `apply` refuses to run
anything whose plan it could not compute.

## 10. Bootstrap

The one piece that cannot be Rust: getting Rust's output onto a bare machine.

```sh
curl -LsSf https://settings.jiun.dev | sh
```

Forty lines of POSIX shell: detect OS and architecture, download the pinned
release for it, verify its SHA-256 against a checksum baked into the script,
install to `~/.local/bin`, `exec kitbag apply`. This is what rustup and uv do,
and it is the only shell that survives.

Releases are built in CI for `aarch64-apple-darwin`, `x86_64-apple-darwin`,
`x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`, published to GitHub
Releases and to the existing `open330/tap`.

## 11. Security

- Secret values are never logged, never printed, never in `--json`. `diff`
  reports key names and value shapes (`len=40`, `ghp_…`), never values. The
  bash version leaked a token exactly once, by classifying key names with a
  denylist; the catalogue here is an **allowlist of what may be shown**.
- Payloads in flight live in a `umask 077` directory removed on exit, including
  on panic. Secrets in memory are zeroized.
- Writing a file backs up what was there first; the vault is never deleted from,
  only reported on.
- `apply` touching system state (`defaults`, `launchd`, shell) requires an
  explicit plan acknowledgement, never a silent default.

## 12. Migration

The settings repo keeps working throughout. The inventory format (manifest v2,
scoped entries) is read and written identically by both, so either tool can run
at any point.

| Phase | What ships | settings repo after |
| --- | --- | --- |
| 0 | this document, repo skeleton, CI | unchanged |
| 1 | `status` `push` `restore` `track` against the existing inventory | `modules/secrets.sh` becomes a 30-line shim; `scripts/secrets-push.sh` deleted |
| 2 | providers: pkg, file, symlink, defaults, launchd, command | modules ported one at a time; `install.sh` dispatches ported ones to kitbag |
| 3 | bootstrap script, release binaries, tap | `install.sh` becomes the 40-line bootstrap |
| 4 | `discover`, `doctor`, TUI, `trust` | `scripts/ssh-trust.sh` deleted |

## 13. Open questions

1. ~~**Recipes: data or code?**~~ **Answered — see appendix A.** Twenty-two
   modules of a working bash installer were classified line by line. Roughly
   four fifths is data once six providers exist; the imperative fifth is almost
   entirely *other people's installers* (rustup, uv, nvm), which belong behind
   an escape hatch permanently rather than in a language of our own.
2. **Where does the public/private line fall for recipes?** A work machine's
   recipe list may itself be sensitive. Probably: recipes public, the machine's
   selection private.
3. **Package removal.** Declaring installed packages is easy; deciding that
   something absent from the file should be uninstalled is how these tools eat
   people's machines. Default: never remove, report drift.
4. **Windows.** WSL is covered by the Linux target. Native Windows is not in
   scope until someone needs it.

---

## Appendix A. What is data and what is not

The recipes were not designed in the abstract. A working bash installer — 22
modules, ~3,500 lines, in daily use on five machines — was read and classified.

### Pure data, once the provider exists

| Module | What it does | Provider |
| --- | --- | --- |
| base | packages, Xcode command line tools | `pkg` |
| cmux, ghostty, hammerspoon, zellij | a cask and a config file | `pkg`, `link` |
| shell, tmux | a package, a config, a plugin manager clone | `pkg`, `link`, `git-clone` |
| fonts | casks plus a pinned download with a checksum | `pkg`, `download` |
| git | an include and a signing setting | `git-config` |
| scripts | a directory of links into `~/.local/bin` | `link` (glob) |
| ssh | configs copied, never symlinked | `copy` (mode-aware) |
| macos | 60-odd `defaults` keys, already held in TSV tables | `defaults` |
| tools | a tool list with a fallback chain per platform | `pkg` (ordered sources) |

That is more than half the modules and most of the lines. Note what made it
easy: the bash version had already pushed its own data out of its code — the
macOS defaults live in tables, the shortcuts in a TSV, the skills in a manifest.
The port is largely reading files that already exist.

### Data, but needing a provider that does not exist yet

| Module | What it needs | Why a provider and not a script |
| --- | --- | --- |
| claude | merge a JSON settings file, link a manifest of skills | merging is the hard part, and it is the same merge every time |
| codex | merge a TOML config | same |
| cship, editor | download a release, verify a checksum, place a binary | the checksum is the point; a script that forgets it is worse than no script |
| node, rust, python | global packages (`npm -g`, `cargo`, `uv`) | a list of names, once the runtime exists |
| editor | "install unless the version is at least X" | a version predicate on `pkg` |

Six providers — `merge`, `download`, `git-clone`, plus `pkg`/`link`/`defaults` —
turn all of this into TOML.

### Genuinely imperative, and staying that way

| What | Why it cannot be data |
| --- | --- |
| rustup, uv, nvm bootstraps | they are *somebody else's installer*, fetched and run. Modelling them would mean tracking their internals forever |
| node via nvm | nvm is a shell function; using it means sourcing a script and inheriting its environment |
| macOS Caps Lock mapping | the value is computed from the keyboards attached right now |
| `pmset`, `launchctl` | sudo, and side effects that are not files |
| hishtory init | a server handshake with a secret |

Five of twenty-two, and four of those five are "run the vendor's installer".

### What follows

Recipes are TOML over typed providers, with one escape hatch:

```toml
[[recipe]]
name = "rust"
[[recipe.command]]
run   = "curl -LsSf https://sh.rustup.rs | sh -s -- -y --no-modify-path"
check = "rustup --version"        # already satisfied? then do nothing
```

`check` is what makes `command` a resource rather than a script: a step with no
way to ask "is this already done" cannot take part in `plan`, and `plan` is the
whole contract. A recipe that cannot answer it is rejected at load time.

No DSL. The moment a recipe wants a conditional, the question to ask is which
provider is missing — the bash version's own history says the answer is usually
"a provider", and only five times in 3,500 lines "a script".
