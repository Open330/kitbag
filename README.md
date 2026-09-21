<div align="center">

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="assets/logo-dark.png">
  <img src="assets/logo.png" width="84" alt="kitbag">
</picture>

# kitbag

**Everything this machine holds that is yours** — what it is, whose it is,
and how it gets onto the next machine.

[![ci](https://github.com/Open330/kitbag/actions/workflows/ci.yml/badge.svg)](https://github.com/Open330/kitbag/actions/workflows/ci.yml)
[![license](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![rust](https://img.shields.io/badge/rust-1.85%2B-orange.svg)](https://www.rust-lang.org)
[![status](https://img.shields.io/badge/status-design-lightgrey.svg)](DESIGN.md)

English · [한국어](README.ko.md)

<img src="docs/demo.gif" width="860" alt="kitbag status">

</div>

```console
$ kitbag status

  personal  (12)
  ├── = env:couchdb              COUCHDB_URI COUCHDB_DB COUCHDB_USER COUCHDB_PASSWORD
  ├── = ssh:id_ed25519           SHA256:XhCQXT9l7zas… (ED25519)
  └── + ssh:authorized_keys      5 keys: laptop desktop mini server phone

  shared · a friend  (4)
  └── = env:llm-proxy            PROXY_URL PROXY_TOKEN MODEL

  work · acme  (17)
  ├── ~ env:ci                   CI_TOKEN CI_URL DEPLOY_KEY_ID …  +5 more
  └── + file:aws-keychain        binary, 25788 bytes → ~/Library/Keychains/…

  mixed  (3)
  └── ? app:accounts             one bundle — personal, work

  + 3 new   ~ 1 changed   = 32 unchanged   ? 1 to build
```

> **Status: early, and working.** Every command below does what it says, against
> a real machine and a real store — `bw`, `op`, `pass` or an `age`-encrypted
> file. What is missing is the installer half: recipes cover packages, links,
> macOS defaults and commands, and the rest of a machine's setup is still
> ahead. See [DESIGN.md](DESIGN.md) for the argument and the plan.

## The question nothing answers

Setting up a machine is two jobs handled by two kinds of tool. Dotfile managers
move your configuration. Password managers move your secrets. Neither can answer
the one that matters once you have more than one machine and more than one
employer:

> **What on this machine belongs to my company, and what happens to it when I
> leave?**

kitbag treats packages, configuration, system settings, credentials and app data
as the same kind of thing — a desired state, a source, a way to apply it, and an
**owner** — and filters every one of them through that owner.

| scope | meaning |
| :-- | :-- |
| `personal` | yours |
| `work` | an employer's or a client's |
| `shared` | an account someone else owns that you were given |
| `mixed` | one artifact holding several — it has to say which |
| `local` | this machine only, never leaves it |

A machine declares which scopes it takes. A personal laptop never restores work
credentials. A work machine does not install your personal toys. The same filter
decides what is sent, what is written, and what a report shows.

## Install

```bash
curl -LsSf https://raw.githubusercontent.com/Open330/kitbag/main/install.sh | sh
```

Detects the platform, checks the download against the checksums published with
it, and puts one binary in `~/.local/bin`. Or `cargo install --path crates/kitbag-cli`.

## Commands

```console
kitbag status                 what this machine has, marked against the store
kitbag plan                   what apply would change, and nothing else
kitbag apply                  make the machine match the recipes
                              (packages, links, defaults, downloads, clones, merges)
kitbag discover               find state nothing is tracking, and what is
                              tracked and not in the store
kitbag track <path> --scope work
kitbag programs               write down what is installed, so it can be again
kitbag push / restore         move it, one scope at a time
kitbag diff                   what differs, without the values
kitbag resolve                settle what neither side can settle alone
kitbag doctor                 permissions, reachability, unscoped files, orphans
kitbag lint                   refuse the things that must not be committed
kitbag trust sync             the machines that may log in here
kitbag completions zsh        …bash, fish, elvish, powershell
```

`--json` on everything, `--color auto|always|never`, `NO_COLOR` respected, and
no command ever prints a secret's value.

## When two machines disagree

A difference has a direction, because kitbag records the fingerprint at the
last exchange — the third point git calls a merge base:

```console
>  this machine moved, the store did not     push sends it
<  the store moved, this machine did not     restore takes it
!  both moved since they agreed              yours to settle
```

A push will not send a `<` and a restore will not take a `>`; both are an
older copy written over a newer one. A `!` stops both and waits:

```console
$ kitbag resolve
! env:docs-publish  (1/2)
    here   3 lines
    store  5 lines
    only there:  DOCS_ROOT DOCS_USER
    differ:      DOCS_URL
    [m]ine  [t]heirs  [s]kip  [q]uit >
```

Key names, counts, sizes and file lists inside an archive — never a value
from either side. An answer that is not understood is a skip, and so is an
empty line: one of the two real answers writes over a credential, so the key
easiest to hit by accident does nothing. With no terminal it asks nothing and
lists what is left.

## Things that belong to one machine

Four machines can derive one item name from one path and hold four different
things under it. An SSH key is the example that matters: two machines sharing
one means revoking it locks out both.

```toml
[[track]]
path = "~/.ssh/id_ed25519"
per_machine = true          # ssh:id_ed25519@<host>
```

Four items, four keys, each one kept — and the file stays at
`~/.ssh/id_ed25519`, where ssh looks for it. Only the name in the store
differs, and a restore leaves alone anything stamped with another machine's.

`skip` is the other answer, for an item this machine wants nothing to do with
in either direction. Not for one that simply belongs to it: refusing to
exchange a key is refusing to back it up, and a key that exists in one place
is gone with the machine it is on.

## Programs, as a list

A store should never hold a binary. It is large, it is built for one
architecture, and whoever published it will hand it over again. What is worth
keeping is what was installed:

```console
$ kitbag programs
kitbag/programs 1
brew	ripgrep
cask	ghostty
cargo	kitbag	0.13.0
npm	@bitwarden/cli	2026.8.0
rustup	stable-aarch64-apple-darwin
```

Seventy-one of those is a kilobyte. Put it back with `kitbag programs
--restore`, which installs what is missing and removes nothing — a machine is
allowed to have more than the list; the list is what it must not lack. A
manager it cannot drive is named rather than guessed at.

Which is fine until the thing you need was never in a manager. `rustup`, `uv`,
`nvm`, and every `curl … | sh` in somebody's setup script are the fifth of a
machine no package list will ever describe, and a list that omits the
toolchain is not one you can rebuild from. So a machine may declare them:

```toml
[[program]]
name = "uv"
install = "curl -LsSf https://astral.sh/uv/install.sh | sh"
version_from = "uv --version"     # optional; the version is read from its output
present = "uv --version"          # optional; defaults to `command -v uv`
```

It is written out only if it is actually there — a declaration nobody has acted
on is a plan, not a fact — and the line travels with the list, so the machine
being rebuilt learns how to install it from the store rather than from a config
it does not have yet. kitbag prints that line before running it, because a
command out of a store is still a command out of a store.

The other machines' lists are readable too, which is the point when the machine
you want to copy is the one that died:

```console
$ kitbag programs --list
jiun-mbp                     programs@jiun-mbp
june-mbp                     programs@june-mbp

$ kitbag programs --from jiun-mbp              # read it
$ kitbag programs --from jiun-mbp --restore    # or become it
```

As a tracked item it is a command pair like any other:

```toml
[[track]]
name = "programs"
scope = "personal"
per_machine = true
command = { export = "kitbag programs", restore = "kitbag programs --restore" }
```

Which is also why `kitbag restore` asks before it writes. Restoring files is
recoverable — each one is backed up first. Installing software is not, so the
whole command stops and asks once; `-y` answers in advance, and `--dry-run`
says exactly what would happen and writes nothing.

## Three stores, and why

A public dotfiles repository is safe only if it leaves out **the list of what
exists**. `~/.envs/kibana.env → work` names an employer, a stack and a target,
and nobody needs the secret to make use of that.

```
  repo (public)        recipes and providers — no employer, no service name
  inventory (private)  what exists, where it goes, which scope — in the store
  secret store         the values — a vault, or an encrypted file
```

Collection is by pattern, never by name. Scope is a marker the file carries
(`# scope: work`), not a table in the repo. `kitbag lint` fails a commit that
breaks either rule — and runs over [this repository](CONTRIBUTING.md) on every
CI run, because a tool that leaked its own author's machine would have argued
against its design.

## Secret stores

kitbag does not implement one. It borrows yours, so that losing interest in
kitbag never strands your secrets inside it.

| backend | store | |
| :-- | :-- | :-- |
| `bw` | Bitwarden / Vaultwarden — free tier, self-hostable | v0.1 |
| `op` | 1Password — the best developer CLI in the category | v0.1 |
| `pass` | `pass` / `gopass` — GPG and a git repo | v0.2 |
| `age` | an `age`-encrypted file — no server at all | v0.2 |

Adding one is a `list`/`get`/`put` adapter. Everything kitbag needs to know
about an item travels inside its own envelope, so a store that can keep bytes
under a name is enough:

```text
kitbag/1
scope: work
owner: acme
encoding: utf8
sha256: 1f0e3d…

export TOKEN=…
```

<details>
<summary>Why there is no <code>delete</code> in the backend trait</summary>

A store holds things your machine knows nothing about — another machine's key,
an account someone else added. A tool that removes what it does not recognise
eventually removes something that mattered. Removal is a person's decision,
taken with the store's own client.

</details>

## Prior art

[chezmoi](https://www.chezmoi.io/) and [yadm](https://yadm.io/) manage dotfiles
and do it well; neither models ownership or moves credentials.
[1Password CLI](https://developer.1password.com/docs/cli/) and
[SOPS](https://github.com/getsops/sops) manage secrets and not the machine.
[Mackup](https://github.com/lra/mackup) moved app state and is unmaintained.
kitbag is the overlap: personal state, wherever it lives, with an owner
attached.

## Building

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Contributions welcome — read [CONTRIBUTING.md](CONTRIBUTING.md) first; the first
rule is that nobody's machine goes in this repository.

<div align="center"><sub>MIT · <a href="DESIGN.md">DESIGN.md</a></sub></div>
