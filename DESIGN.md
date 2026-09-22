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
kitbag backup                 set this machine up and send what it holds, asking as it goes
kitbag status                 what this machine has, grouped by scope, marked against the store
kitbag plan                   what apply would change
kitbag apply [--only pkg]     make it so
kitbag discover               scan for personal state that is not tracked yet, and propose it
kitbag catalogue              what discover looks for, and where to add to it
kitbag add <path>…            start keeping it — the `git add` of this tool
kitbag tracked                what this machine was told to keep, as it was told
kitbag push [--only name…]    send tracked state to the store
kitbag restore [--only name…] write it back here (asks first; -y to skip)
kitbag diff [name…]           what differs from the store — shapes and names, never values
kitbag resolve                settle, one at a time, what neither side can settle alone
kitbag programs [--restore]   what is installed here, written down; or put back from that writing
kitbag doctor                 permissions, reachability, unscoped files, orphans in the store
kitbag trust                  the machines that may log in here (folds in ssh-trust.sh)
```

Defaults that matter: `plan` is implied unless `apply`/`push`/`restore` is asked
for, output is a tree grouped by scope, `--json` exists for every command, and
no command ever prints a secret's value.

`restore` asks before it writes, and `-y` answers in advance. It did not, once,
and that was defensible while restoring meant writing files — those are backed
up first and a backup can be put back. A `programs` item made restore able to
install software, which reaches the network, takes minutes, and no backup
undoes. What it does not do is print a plan to confirm against: building one
means fetching every payload to find out, which is twice the calls and every
secret held twice as long, for a question `--dry-run` already answers.

`tracked` and `status` answer different questions, and the difference is the
point: `status` shows the items a track came to, `tracked` shows the line
somebody wrote. It is the only place a filter's effect is visible — `status`
shows what came through, and silence about the rest reads as "there was
nothing else" — and the only place a tracked path with no file behind it says
so, which is the quietest way to believe in a backup that does not exist.

`add` exists because saying "keep this" should not be an edit. The config file
is there so a machine can remember the answer, not because a person should have
to type it in that shape. What it works out rather than asking — and prints, so
none of it is silent: a directory becomes the pattern that covers the files not
in it yet; a directory holding both scripts and installed binaries gets
`only = "scripts"`; and a file carrying its own `# scope:` marker gets no second
answer written beside it. `--everywhere` writes the catalogue rule too.

`backup` is the others in an order: `discover`, `track`, `push` and, if
anything was held, `resolve`. It exists because needing four commands in the
right sequence — one of which lived in a different repository — is a way of
saying the tool is for people who already know how it works. It runs nothing
the named commands do not, which is the point: a walkthrough that did its own
thing would be a second implementation to keep honest.

Four global flags decide what a run touches: `--scope` (which scopes this
machine takes this time), `--skip name,…` (items it keeps to itself, in both
directions), `--jobs N` (how many items are in flight at once — each is a
process, and starting them one after another is most of the wait), and
`--backend`.

```
  work · acme  (14)
  ├── > env:github               GITHUB_GITEA_TOKEN GITHUB_DEPLOY_TOKEN …
  ├── = env:jenkins              JENKINS_API_KEY JENKINS_USER
  └── + file:aws-vault-keychain  binary, 25788 bytes → ~/Library/Keychains/…

  + 3 new   > 1 ahead   = 35 unchanged   ? 1 not comparable
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

enum State {
    New,        // here, and the store has never seen it
    Ahead,      // moved here since the last exchange; the store stayed put
    Behind,     // the store moved; this machine stayed put
    Conflict,   // both moved, and not to the same place
    Unchanged,  // neither moved
    Unknown,    // only building it would tell — see `volatile` below
}
```

Two rules the bash version arrived at the hard way, kept as invariants:

- **A marker travels with the file.** A `# scope:` header inside a file wins
  over any table that names it, because the table does not get copied along.
- **Merging never deletes.** A host may hold keys, lines or accounts this tool
  has never seen. Only an explicit `revoke`/`prune` removes anything.

Four more that the first four machines added:

- **A path is not a name.** Every machine's ssh key lives at
  `~/.ssh/id_ed25519`; four of them must not overwrite each other on the way
  into one store. A `per_machine` track appends `@<machine>` to the *item name*
  and puts the machine in the envelope. The path is left exactly where it was:
  the difference exists in the store and nowhere else.
- **A refusal belongs to the machine making it.** `skip` names items this
  machine neither sends nor accepts. It is read from the machine's own config,
  never from the store, so a machine cannot be talked into taking something it
  has declined — and it works in both directions, because the dangerous half is
  the one that writes.
- **Not every export is stable.** An export carrying a timestamp or a window
  position differs from itself between two runs. `volatile` says so: the item is
  still sent, and it is reported `?` rather than accused every day of having
  changed.
- **A program is a name, not a payload.** Binaries are large, built for one
  architecture, and still available from whoever published them. What is worth
  keeping is `manager, name, version`; a restore asks the manager for it again.
  Nothing is ever uninstalled — the list is what a machine must not lack, not
  what it may not exceed. What no manager will ever list — `rustup`, `uv`, every
  `curl … | sh` — is declared with the line that installs it, and that line
  travels with the list, because the machine that has to run it is the one
  being rebuilt.
- **A declaration is not an installation.** A declared program is written into
  the list only if it is actually on the machine. Otherwise the list stops
  describing what is here and starts describing what somebody intended.

### 5.1 Three-way, because two-way cannot say who moved

Comparing this machine against the store answers "are these the same" and
nothing further. It cannot tell a local edit from a remote one, so every
difference arrives as a question for a person. The missing third point is what
the two last agreed on:

```
~/.config/kitbag/exchanged     name → fingerprint, mode 0600
```

Every push and every restore records the fingerprint of what crossed. After
that:

| here | store | last agreed | state |
| --- | --- | --- | --- |
| a | a | a | `=` unchanged |
| b | a | a | `>` ahead — `push` sends it |
| a | b | a | `<` behind — `restore` writes it |
| b | c | a | `!` conflict — only a person can say |
| b | c | — | `~` changed with no base, treated as a conflict |

`push` sends what is ahead and holds the conflicts; `restore` writes what is
behind and holds the conflicts; neither picks a winner. `resolve` then walks
what was held, one item at a time, showing what differs before it asks —
structurally for keys, by line for text, by entry for archives, and by size
alone for anything holding key material, where the shape of the difference is
itself worth not printing.

Two hashes, because the two questions are not the same one. `sha256` covers the
payload alone, so a restore can compare it against a file already on disk. The
`fingerprint` covers the whole envelope, so a push notices that the scope, the
machine or the platform changed even when the bytes did not.

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

**The catalogue covers two kinds of thing, and says which.** Credentials, and
*setup* — shell profiles, editor configuration, and the scripts in `~/bin` and
`~/.local/bin`. A machine restored with every secret intact and no shell
profile is one somebody still has to spend an evening on, and the part a
settings repository usually carries in git is exactly the part that was
missing.

Two rules make that safe rather than merely broad:

- **A `bin` directory holds what somebody wrote and what a package manager
  installed.** `only = "scripts"` takes the files beginning `#!`. The rest are
  binaries built for one architecture — what the `programs` list carries as a
  name instead. An `only` this version does not recognise takes nothing and
  says so, because a filter that silently degrades to "everything" sends what
  somebody asked to have filtered out.
- **A file a git repository already holds is named, not proposed.** A settings
  repository symlinks `~/.zshrc` into itself. Asked of every file a pattern
  matches rather than the first, since a directory mixing linked and unlinked
  scripts is the ordinary case.

**And the list is open.** `~/.config/kitbag/catalogue.toml` adds entries in the
same shape, and one naming a path the built-in list already has replaces it —
correcting a guess should not mean arguing with a constant somebody else
compiled. A file that will not parse is reported with the built-in list still
in use, and an unknown key is an error: a catalogue that quietly does nothing,
or quietly does something else, is how somebody comes to believe they are
watching a path they are not.

Each finding comes with a proposed scope and the reason for it, and is accepted
or dismissed interactively. Dismissals are remembered, so the second run is
quiet.

Tracked is not the same as kept, and the difference is the one that costs
something. A path can sit in this machine's config for months, named and
scoped and never once sent, and every report will call it tracked. So
`discover` asks the store as well, and says which is which:

```text
1 tracked here and not in the store:
! env:two                      ~/.envs/two.env
kitbag push --backend <name> sends them.
```

It also proposes what has no path at all. A machine with a package manager and
nothing recording one is missing a `programs` track, and that is as discoverable
as a file is.

## 9. Architecture

```
crates/
  kitbag-core      model, scope, plan/apply engine, hashing, diff
  kitbag-providers pkg(brew|apt|cask|mas|cargo|npm) file defaults launchd command programs
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
path: ~/.envs/acme.env
platform: macos
machine: june-mbp
restore: file
encoding: utf8
sha256: 1f0e3d…

export TOKEN=…
```

The headers past `scope` are what makes a store into something a machine can be
rebuilt from: where the payload goes, what writes it, which platform it was
taken on, and — for a `per_machine` item — whose it is. A machine reading an
item for another platform declines it rather than writing a Windows path onto a
Mac.

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

Where it actually stands: phase 1 is done and past the interesting part.
`modules/secrets.sh` dispatches to kitbag, four machines run it, and the shell
engine remains only as the fallback path and as the thing the Windows machine
still uses. The two engines share one vault and — this is the part that bites —
share item *names* too, so every reader filters on a `kitbag` field to tell a
kitbag item from its shell-era namesake. Phase 2 has `programs`; the rest of it
is still bash.

## 13. Open questions

1. ~~**Recipes: data or code?**~~ **Answered — see appendix A.** Twenty-two
   modules of a working bash installer were classified line by line. Roughly
   four fifths is data once six providers exist; the imperative fifth is almost
   entirely *other people's installers* (rustup, uv, nvm), which belong behind
   an escape hatch permanently rather than in a language of our own. That hatch
   is `[[program]]`: a name and the line that installs it, recorded in the
   machine's own config and carried with the list.
2. **Where does the public/private line fall for recipes?** A work machine's
   recipe list may itself be sensitive. Probably: recipes public, the machine's
   selection private.
3. ~~**Package removal.**~~ **Answered in the only direction that is safe.**
   `programs` records what is installed and installs back what is missing;
   absence from the list removes nothing, ever. Deciding that something absent
   from a file should be uninstalled is how these tools eat people's machines.
   Drift is reported, not corrected.
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
| aas, otpeek | state only the application can hand over | a command pair: `export` to stdout, `restore` from stdin |
| codex | merge a TOML config | same |
| cship, editor | download a release, verify a checksum, place a binary | the checksum is the point; a script that forgets it is worse than no script |
| node, rust, python | global packages (`npm -g`, `cargo`, `uv`) | a list of names, once the runtime exists |
| editor | "install unless the version is at least X" | a version predicate on `pkg` |

Six providers — `merge`, `download`, `git-clone`, plus `pkg`/`link`/`defaults` —
turn all of this into TOML. **All six exist**, along with the `command` escape
hatch; what remains unported is the handful of modules in the next table.

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
