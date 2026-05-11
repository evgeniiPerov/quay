# CLI overview

`quay` is a single binary with sixteen subcommands. Every subcommand respects four global options:

| Flag | Effect |
|---|---|
| `--project <PATH>` | Treat `<PATH>` as the project root instead of `cwd`. |
| `--user-config <PATH>` | Override user config (default: `~/.config/quay/config.toml`). |
| `--profile <NAME>` | Override active profile for this invocation. `QUAY_PROFILE` env var works too. |
| `--json` | Emit machine-readable JSON on stdout instead of a human table. |

## Subcommand groups

**Project setup**

- [`init`](init.md) — drop a `.quay/config.toml` skeleton into the current project.
- [`remote`](remote.md) — add / list / test / remove configured hub remotes.
- [`profile`](profile.md) — manage user profiles (work + personal on one machine).

**Consume**

- [`add`](add.md) — install a skill from a hub.
- `list` — list currently installed skills (no dedicated page; covered briefly below).
- [`info`](info.md) — show metadata for a skill without installing.
- [`search`](search.md) — free-text search across configured remotes.
- [`outdated`](outdated.md) — list installed skills with newer hub versions.
- [`update`](update.md) — pull newer versions over installed skills.
- [`remove`](remove.md) — uninstall a skill (optionally `--everywhere`).
- [`link`](link.md) — manage mirror dirs (`.claude/`, `.cursor/`, …).

**Author / publish**

- [`scan`](scan.md) — discover local skills and their sync status.
- [`validate`](validate.md) — check frontmatter offline.
- [`push`](push.md) — publish a skill to a hub.
- [`rebuild-registry`](rebuild-registry.md) — regenerate hub `registry.json` from disk.

**Other**

- [`tui`](tui.md) — launch the interactive terminal UI.

## `quay list`

Prints every installed skill in the current project plus its version and source remote. Common flags only (`--profile`, `--user-config`, `--project`, `--json`). Use it as a sanity check after `quay add`, or to feed `xargs` in scripts:

```sh
quay list --json | jq -r '.[].name' | xargs -n1 quay info
```

## Exit codes

See [Exit codes](../reference/exit-codes.md) for the full table. Quick reference:

| Code | Meaning |
|---|---|
| 0 | Success |
| 1 | Generic command failure |
| 2 | `clap` parse error (bad flags / args) |

## `--json` example

```sh
quay list --json | jq '.[] | {name, version}'
```

Use this for piping into other tools. Human-table output is not stable; the JSON shape is.

## Top-level help

```text
Skill registry CLI

Usage: quay [OPTIONS] <COMMAND>

Commands:
  init              Initialize quay in the current project
  remote            Manage configured hub remotes
  add               Install a skill from a configured remote
  list              List installed skills
  remove            Remove a previously installed skill
  info              Show metadata for a skill (without installing)
  search            Search across configured remotes
  outdated          List installed skills that have newer versions available
  update            Update installed skills to the latest available version
  scan              Discover local skills under `.agents/skills/` and report their sync status
  validate          Validate a local skill's frontmatter (offline, no network)
  push              Push a local skill to a hub via PR (or directly, if --push-mode=direct or the remote's TOML says so)
  profile           Manage user profiles (multi-org identities + remote bundles)
  rebuild-registry  Rebuild a hub's `registry.json` from disk truth and push it back
  link              Apply or verify mirrors from `[install].mirrors` config
  tui               Launch the interactive TUI
  help              Print this message or the help of the given subcommand(s)
```
