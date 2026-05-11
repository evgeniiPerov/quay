# `quay outdated`

List installed skills that have a newer version available on the hub.

## Usage

```text
quay outdated [OPTIONS]
```

Read-only sibling of `quay update`. Detects two things:

1. **Version drift** — hub's frontmatter `version` is semver-greater than the installed copy's.
2. **SHA drift on installed-modified** — the local file SHA differs from the install-time SHA on the lockfile entry, regardless of version.

Both are reported with a status badge.

## Examples

```sh
quay outdated
quay outdated --json | jq '.[].name'
```

## Flags

| Flag | Effect |
|---|---|
| `--profile`, `--user-config`, `--project`, `--json` | Standard globals. |

## When to use this vs …

- [`quay update`](update.md) `--dry-run` — same detection, but `update` can also apply the fix. Use `outdated` when you want a non-interactive read-only report (CI, status dashboards).
- [`quay scan`](scan.md) — broader local audit; reports all four statuses (`local`, `installed`, `installed-modified`, `pushed-local`).
- TUI **Dashboard** "Outdated" panel — same data, visual.

## Caveats

- "Outdated" by version only works for Frontmatter skills. Other formats are compared SHA-only.
- A hub with no `registry.json` produces a warning and counts as "everything's current."

## `--help`

```text
List installed skills that have newer versions available

Usage: quay outdated [OPTIONS]

Options:
      --project <PROJECT>          Project root (defaults to current directory)
      --user-config <USER_CONFIG>  Override user config path (defaults to ~/.config/quay/config.toml)
      --profile <PROFILE>          Override the active profile for this invocation
      --json                       Output JSON instead of human-readable text
  -h, --help                       Print help
```
