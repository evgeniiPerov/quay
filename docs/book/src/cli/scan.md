# `quay scan`

Discover local skills and report their sync status.

## Usage

```text
quay scan [OPTIONS]
```

Walks `.agents/skills/` one level deep, classifies every directory it finds, and prints a table. Cross-references the project lockfile and `~/.config/quay/push-log.json` to compute per-skill status:

| Status | Meaning |
|---|---|
| `local` | Authored here; never pushed. |
| `installed` | Came from a hub via `quay add`. |
| `installed-modified` | Came from a hub, but local file has been edited since install. |
| `pushed-local` | Authored here, then pushed (so it exists on a hub too). |

## Examples

```sh
quay scan
quay scan --json | jq '.[] | select(.status == "installed-modified")'
quay scan --root ./vendor/skills        # non-default canonical root
```

## Flags

| Flag | Effect |
|---|---|
| `--root <PATH>` | Override the canonical skills root (default: `<project>/.agents/skills`). |
| `--json` | JSON output (also accepted as `--json` global). |
| `--profile`, `--user-config`, `--project` | Standard globals. |

## When to use this vs …

- [`quay outdated`](outdated.md) — only reports drift against the hub. `scan` reports authorship + drift in one pass.
- `quay list` — pre-scan command; shows installed skills only, no local-authored entries.
- TUI **Dashboard** "Local skills" panel — same data, visual.

## Caveats

- A skill with no recognisable format still appears (status `local`, format `Freestyle`). `scan` never errors on weird files; it just classifies.
- `--root` affects discovery but does **not** rewrite the project config; if you regularly use a non-default root, set it in `.quay/config.toml` instead.

## `--help`

```text
Discover local skills under `.agents/skills/` and report their sync status

Usage: quay scan [OPTIONS]

Options:
      --project <PROJECT>          Project root (defaults to current directory)
      --root <ROOT>                Override the install canonical root (default: `<project>/.agents/skills`)
      --json                       Emit JSON instead of a human table
      --user-config <USER_CONFIG>  Override user config path (defaults to ~/.config/quay/config.toml)
      --profile <PROFILE>          Override the active profile for this invocation
  -h, --help                       Print help
```
