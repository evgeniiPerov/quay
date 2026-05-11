# `quay remove`

Uninstall a previously installed skill.

## Usage

```text
quay remove [OPTIONS] [SKILL]
```

Default: removes only the local copy under `.agents/skills/<name>/` and any mirror dirs. The hub is untouched. Pass `--everywhere` to also delete the skill from every configured remote that publishes it.

## Examples

```sh
quay remove hello                   # local only
quay remove hello --everywhere      # also drop from every hub
quay remove -i                      # interactive picker
```

## Flags

| Flag | Effect |
|---|---|
| `--everywhere` | Also commit a deletion to every remote publishing this skill (respects per-remote `push_mode`). |
| `-i, --interactive` | Pick from installed skills. |
| `--profile`, `--user-config`, `--project`, `--json` | Standard globals. |

## When to use this vs …

- [`quay update`](update.md) — to refresh, not uninstall.
- TUI **Local** screen + `[d]` (single) / `[D]` (bulk selected) — same flow with a UI.
- [`quay link`](link.md) `remove <path>` — remove a *mirror directory* from the project config, not a skill.

## Caveats

- `--everywhere` deletes from the remote without a confirmation prompt. Combined with `push_mode = "direct"` it's a destructive one-shot. Use PR mode if you want a review gate.
- Removing locally does not edit `~/.config/quay/push-log.json` — the push history is preserved for audit.
- If a skill was installed via mirror dir only (rare), `remove` still cleans up the canonical `.agents/skills/<name>/` path.

## `--help`

```text
Remove a previously installed skill

Usage: quay remove [OPTIONS] [SKILL]

Arguments:
  [SKILL]  Skill name to remove. Omit when using --interactive (-i)

Options:
      --everywhere                 Also push a deletion commit to every remote that publishes the skill
      --project <PROJECT>          Project root (defaults to current directory)
  -i, --interactive                Open an interactive checkbox list to pick skills to remove. Mutually exclusive with the positional skill argument
      --user-config <USER_CONFIG>  Override user config path (defaults to ~/.config/quay/config.toml)
      --profile <PROFILE>          Override the active profile for this invocation
      --json                       Output JSON instead of human-readable text
  -h, --help                       Print help
```
