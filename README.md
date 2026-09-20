# kitbag

> Everything this machine holds that is yours: what it is, whose it is, and how
> it gets onto the next machine.

**Status: design.** The model and the command surface are written down; the
engine behind them is not built yet. See [DESIGN.md](DESIGN.md).

## The idea

Setting up a new machine is two jobs that are usually done by two kinds of tool.
Dotfile managers move your configuration. Password managers move your secrets.
Neither can answer the question that actually matters once you have more than
one machine and more than one employer:

> What on this machine belongs to my company, and what happens to it when I
> leave?

kitbag treats packages, configuration, system settings, credentials and app data
as the same kind of thing — a desired state, a source, a way to apply it, and an
**owner** — and filters all of it through one idea:

| scope | meaning |
| --- | --- |
| `personal` | yours |
| `work` | an employer's or a client's |
| `shared` | an account someone else owns that you were given |
| `mixed` | one artifact holding several — it must say which |
| `local` | this machine only, never leaves it |

A machine declares which scopes it takes. A personal laptop never restores work
credentials. A work machine does not install your personal toys. The same filter
decides what is sent, what is written, and what a report shows.

```
  work · acme  (14)
  ├── ~ env:github               GITHUB_GITEA_TOKEN GITHUB_DEPLOY_TOKEN …
  ├── = env:jenkins              JENKINS_API_KEY JENKINS_USER
  └── + file:aws-vault-keychain  binary, 25788 bytes → ~/Library/Keychains/…

  + 3 new   ~ 1 changed   = 35 unchanged   ? 1 built on push
```

## Three stores, and why

A public dotfiles repository is safe only if it does not carry **the list of
what exists**. `~/.envs/kibana.env → work` names an employer, a stack, and a
target. So kitbag splits them:

```
  repo (public)        recipes and providers — no employer, no service name
  inventory (private)  what exists, where it goes, which scope — in the store
  secret store         the values — a vault, or an age-encrypted file
```

Collection is by pattern, never by name. Scope is a marker the file carries
(`# scope: work`), not a table in the repo. `kitbag lint` fails a commit that
breaks either rule.

## Secret stores

kitbag does not implement one — it borrows yours, so that losing interest in
kitbag does not strand your secrets inside it.

| backend | store | status |
| --- | --- | --- |
| `bw` | Bitwarden / Vaultwarden — free tier, self-hostable | v0.1 |
| `op` | 1Password — the best developer CLI in the category | v0.1 |
| `pass` | `pass` / `gopass` — GPG and a git repo | v0.2 |
| `age` | an `age`-encrypted file — no server at all | v0.2 |

Adding one is a `list`/`get`/`put` adapter. Everything kitbag needs to know
about an item — scope, owner, payload hash — travels inside its own envelope,
so a store that can keep bytes under a name is enough:

```text
kitbag/1
scope: work
owner: acme
encoding: utf8
sha256: 1f0e3d…

export TOKEN=…
```

There is no `delete` in the backend trait. A store holds things your machine
knows nothing about; a tool that removes what it does not recognise eventually
removes something that mattered.

## Prior art

[chezmoi](https://www.chezmoi.io/) and [yadm](https://yadm.io/) manage dotfiles
and do it well; neither models ownership or moves credentials.
[1Password CLI](https://developer.1password.com/docs/cli/) and
[SOPS](https://github.com/getsops/sops) manage secrets and not the machine.
[Mackup](https://github.com/lra/mackup) moved app state and is unmaintained.
kitbag is the overlap: the state that is personal, wherever it lives, with an
owner attached.

## License

MIT
