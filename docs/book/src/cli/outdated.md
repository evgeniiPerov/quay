# `quay outdated`

List installed skills that have a newer version available on the hub.

## Usage

```text
quay outdated [OPTIONS]
```

Read-only sibling of `quay update`. Detects two things:

1. **Version upgrade** — the hub's frontmatter `version` is semver-greater than the installed copy's. Reported as `local -> <version>`.
2. **Content drift** — the local bytes differ from the hub's published `content_hash`, whatever the versions say. Reported as `differs from hub at <version>`.

Content drift matters because bumping `version` on push is a convention, not something quay enforces. A hub maintainer who fixes a typo or edits `references/` has no reason to touch `version`, and a semver-only comparison would call your stale copy up to date. Hand-written skills (`SlashCommand`, `Freestyle`) have no semver at all, so drift is the only signal they ever produce.

Drift is **direction-neutral**: two hashes can prove the bytes differ, not which side changed them. Either the hub was edited or you were. `quay update` acts on version upgrades only, so a drift row is taken with:

```sh
quay add <name> --force
```

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

- [`quay update`](update.md) `--dry-run` — overlapping but narrower: `update` acts on version upgrades only, so it will not offer to resolve a content-drift row. Use `outdated` when you want a non-interactive read-only report (CI, status dashboards).
- [`quay scan`](scan.md) — broader local audit; reports all four statuses (`local`, `installed`, `installed-modified`, `pushed-local`).

## Caveats

- Version comparison only applies to Frontmatter skills. Other formats have no semver and are compared by content hash alone.
- A hub whose `registry.json` predates content-hash indexing publishes no `content_hash`. Drift cannot be computed there, and is never reported — a missing hash is not evidence of a match.
- Line endings are normalized before comparing, so a Windows checkout (`core.autocrlf` rewrites LF to CRLF) does not report every skill as drifted. This normalizes the local side only: a hub that published a hash computed from CRLF bytes still mismatches an LF checkout.
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
