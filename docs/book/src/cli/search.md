# `quay search`

Free-text search across configured remotes.

## Usage

```text
quay search [OPTIONS] <QUERY>
```

Matches `<QUERY>` case-insensitively against skill `name` and `description` in every reachable hub's `registry.json`. Add `--tag` to narrow by tag.

## Examples

```sh
quay search lint
quay search "react component" --tag frontend
quay search foo --remote personal
quay search --json deploy | jq '.[].name'
```

## Flags

| Flag | Effect |
|---|---|
| `--remote <NAME>` | Search only one remote. Default: all configured. |
| `--tag <TAG>` | Restrict to skills whose `tags` include `<TAG>`. Repeat not supported. |
| `--profile`, `--user-config`, `--project`, `--json` | Standard globals. |

## When to use this vs …

- [`quay info`](info.md) — when you already know the name, get full metadata.
- [`quay scan`](scan.md) — local-side equivalent; lists *your* skills, not the hubs'.

## Caveats

- Search hits a cached `registry.json` per remote. If the hub was just pushed to seconds ago, you may get a stale result — `?_cb` cache-busting in the fetcher mitigates this, but five-minute CDN tail can still bite. Retry after a minute.
- A hub with a malformed `registry.json` is skipped silently (logged to stderr); the rest of the search continues.
- Empty results print a clean "no results" line and exit 0.

## `--help`

```text
Search across configured remotes

Usage: quay search [OPTIONS] <QUERY>

Arguments:
  <QUERY>  Free-text query matched against skill name + description (case-insensitive)

Options:
      --project <PROJECT>          Project root (defaults to current directory)
      --remote <REMOTE>
      --tag <TAG>
      --user-config <USER_CONFIG>  Override user config path (defaults to ~/.config/quay/config.toml)
      --profile <PROFILE>          Override the active profile for this invocation
      --json                       Output JSON instead of human-readable text
  -h, --help                       Print help
```
