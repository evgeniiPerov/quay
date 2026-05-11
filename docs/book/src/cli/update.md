# `quay update`

Update installed skills to the latest hub version.

## Usage

```text
quay update [OPTIONS] [SKILL]
```

Omit `SKILL` to update every installed skill that has a newer version on the hub. Inside a TTY this auto-opens a checkbox picker of outdated skills; pass `--all` to bypass the picker, or `--dry-run` to preview.

## Examples

```sh
quay update                          # TTY: opens picker of outdated. Non-TTY: updates all.
quay update hello                    # just this one
quay update --all                    # all outdated, no picker
quay update --dry-run                # show what would change
quay update -i                       # explicit picker
```

## Flags

| Flag | Effect |
|---|---|
| `--dry-run` | Show what would change without writing. |
| `-i, --interactive` | Open the picker. |
| `--all` | Skip the TTY auto-picker; update everything outdated. |
| `--profile`, `--user-config`, `--project`, `--json` | Standard globals. |

## When to use this vs …

- [`quay outdated`](outdated.md) — list-only sibling; identical detection logic without writing anything.
- [`quay add`](add.md) `<skill> --force` — overwrites local drift even when versions match.
- TUI **Local** screen — outdated skills show with a colour badge; `[u]` pushes your local, no in-place update key (use CLI).

## Caveats

- "Outdated" uses semver comparison on the frontmatter `version` field. Skills without frontmatter (SlashCommand / Freestyle) never show as outdated by version — they're compared by SHA-on-fly instead.
- Local edits are detected and surfaced: an outdated skill with local modifications is reported as `installed-modified`. `update` refuses to overwrite it without `--force` (which lives on `quay add`, not `update` — use `quay add <name> --force` to nuke).
- `--dry-run` does not query CLI tools (`gh` / `glab`); pure diff against hub clone.

## `--help`

```text
Update installed skills to the latest available version

Usage: quay update [OPTIONS] [SKILL]

Arguments:
  [SKILL]  Update only this skill; if omitted, updates every installed skill

Options:
      --dry-run                    Show what would change without writing to disk
      --project <PROJECT>          Project root (defaults to current directory)
  -i, --interactive                Open an interactive checkbox list of outdated skills to update. Mutually exclusive with the positional skill argument
      --user-config <USER_CONFIG>  Override user config path (defaults to ~/.config/quay/config.toml)
      --all                        Update every installed skill without opening the picker, even in a terminal. Explicit bypass for the TTY auto-trigger
      --profile <PROFILE>          Override the active profile for this invocation
      --json                       Output JSON instead of human-readable text
  -h, --help                       Print help
```
