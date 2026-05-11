# `quay profile`

Manage user profiles — bundles of identity (commit email) + remotes + install settings.

## Usage

```text
quay profile <SUBCOMMAND>
```

Subcommands:

| Subcommand | What |
|---|---|
| `list` | Show all profiles. Active one is marked. |
| `current` | Print the active profile name. |
| `add` | Create a new profile (three modes — see below). |
| `use` | Switch the active profile. `-i` for interactive picker. |
| `remove` | Delete a profile (cannot remove the last one). |
| `show` | Print full profile contents (TOML-shaped). |
| `rename` | Rename a profile. |

## The three `add` modes

1. **Wizard** — `quay profile add -i`. Walks you through name, email, hub URL, push_mode. Recommended for first-time setup.
2. **Explicit flags** — `quay profile add work --email me@corp --remote main=git@github.com:corp/skills.git --provider github --push-mode pr --default --activate`. Repeatable `--remote` to seed several; `--provider` / `--push-mode` / `--default` apply to the most recent `--remote`.
3. **From TOML** — `quay profile add ci --from-toml /etc/quay/ci.toml` (or `-` for stdin). Lets you commit profile fragments alongside CI configs.

## Examples

```sh
quay profile add -i                            # wizard
quay profile add work --email me@corp \
  --remote main=git@github.com:corp/skills.git \
  --provider github --push-mode pr --default --activate
quay profile add ci --from-toml ./ci-profile.toml
quay profile use -i                            # picker
quay profile use personal
quay profile show work
quay profile rename work corp
quay profile remove old-profile
```

## Flags

Common globals apply (`--project`, `--user-config`, `--profile`, `--json`). The `add` subcommand carries the most flags — see `quay profile add --help`.

## When to use this vs …

- [`quay remote add`](remote.md) — adds a remote to the *currently active* profile. `profile add` creates the profile + can seed remotes in one go.
- `QUAY_PROFILE` env var — one-shot override at invocation time. `profile use` makes the switch persistent.
- TUI **Settings → Profiles** (`,` then **Profiles** tab) — same flow with a UI.

## Caveats

- The "last profile" cannot be removed. quay always has at least one profile.
- `--activate` only matters during `add`. Use `quay profile use <name>` afterwards to switch.
- A profile without any remotes is valid but unhelpful — most commands will error with "no remotes configured." Add one with `quay remote add`.

## `--help` (top level)

```text
Manage user profiles (multi-org identities + remote bundles)

Usage: quay profile [OPTIONS] <COMMAND>

Commands:
  list     List all profiles, marking the active one
  current  Print the active profile name
  add      Add a new profile
  use      Set the active profile
  remove   Remove a profile (cannot remove the last one)
  show     Print full profile contents
  rename   Rename a profile
  help     Print this message or the help of the given subcommand(s)
```
