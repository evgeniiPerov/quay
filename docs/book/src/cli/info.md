# `quay info`

Show metadata for a skill on the hub, without installing.

## Usage

```text
quay info [OPTIONS] <SKILL>
```

Fetches the hub's `registry.json`, locates the entry for `<SKILL>`, prints `name`, `description`, `version`, `tags`, source format, and the canonical path.

## Examples

```sh
quay info hello
quay info hello --remote work
quay info hello --json | jq '.description'
```

## Flags

| Flag | Effect |
|---|---|
| `--remote <NAME>` | Use a specific remote. Default: profile default. |
| `--profile`, `--user-config`, `--project`, `--json` | Standard globals. |

## When to use this vs …

- [`quay search`](search.md) — discovery; use `info` once you have a name.
- [`quay add`](add.md) — when you actually want it installed.
- TUI **Remote** screen — `info`-equivalent preview pane next to the skill list.

## Caveats

- Returns an error if `<SKILL>` is not in the registry. Possible causes: typo, hub's `registry.json` is stale (run [`quay rebuild-registry`](rebuild-registry.md)), or the skill was added without updating the registry (every modern `quay push` updates it; older flows didn't).
- Does not download the skill body — just the registry entry. To read the full `SKILL.md`, install it with `--force` to a throwaway dir, or browse the hub on GitHub.

## `--help`

```text
Show metadata for a skill (without installing)

Usage: quay info [OPTIONS] <SKILL>

Arguments:
  <SKILL>

Options:
      --project <PROJECT>          Project root (defaults to current directory)
      --remote <REMOTE>
      --user-config <USER_CONFIG>  Override user config path (defaults to ~/.config/quay/config.toml)
      --profile <PROFILE>          Override the active profile for this invocation
      --json                       Output JSON instead of human-readable text
  -h, --help                       Print help
```
