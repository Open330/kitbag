<div align="center">

<img src="assets/logo.svg" width="88" alt="kitbag">

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

> **Status: design.** The model, the command surface and the guardrails exist
> and are tested. The engine behind them is not built yet — see
> [DESIGN.md](DESIGN.md) for the argument and the plan.

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

## Commands

```console
kitbag status                 what this machine has, marked against the store
kitbag plan                   what apply would change, and nothing else
kitbag apply                  make the machine match the recipes
kitbag discover               find personal state nothing is tracking yet
kitbag track <path> --scope work
kitbag push / restore         move it, one scope at a time
kitbag doctor                 permissions, reachability, unscoped files, orphans
kitbag lint                   refuse the things that must not be committed
kitbag completions zsh        …bash, fish, elvish, powershell
```

`--json` on everything, `--color auto|always|never`, `NO_COLOR` respected, and
no command ever prints a secret's value.

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
