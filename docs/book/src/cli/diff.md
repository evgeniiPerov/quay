# `quay diff`

Show how a locally installed skill differs from the hub's copy. Read-only — it never writes to your project.

## Usage

```text
quay diff <SKILL> [OPTIONS]
```

`quay outdated` tells you *that* something differs. This tells you *what*, file by file.

## What it compares

The whole skill **directory**, not just `SKILL.md`. A hub edit to `references/api.md` or `scripts/run.sh` shows up here; comparing only `SKILL.md` would report such a skill as identical.

The file set matches what `quay push` sends — dotfiles, dotdirs and symlinks are excluded on both sides, so a `.gitkeep` living on the hub is not reported as a difference no `quay add` could resolve.

## Verdicts

| Verdict | Meaning |
|---|---|
| `up to date` | Your copy and the hub's are byte-identical. |
| `hub is ahead by N commit(s)` | Your copy matches an earlier hub commit, and the hub has moved on since. |
| `your copy is ahead of the hub` | The hub's HEAD is an ancestor of the commit your copy matches. |
| `differs …, direction is unknown` | No hub commit matches your bytes. Usually means you edited locally, but a squashed or rewritten hub history looks the same. |
| `no longer on the hub` | The skill directory does not exist at the hub's HEAD — deleted or renamed upstream. |

The verdict comes from history, not from version numbers. A frontmatter `version` relation is printed as an advisory line when the two sides disagree, and never decides the verdict: a hub can publish new content at an unchanged version, and your copy can carry a higher version than the hub while holding older content.

## Reading the diff

Diffs are **pull-oriented**: `+` is what the hub would give you, `-` is what you have now. (`quay add`'s collision prompt renders the opposite direction, because there the question is what you would push.)

Per-file markers:

| Mark | Meaning |
|---|---|
| `M` | Present on both sides, contents differ. |
| `+` | On the hub only — a `quay add --force` would add it. |
| `-` | Local only — the hub does not have this file. |

Binary (non-UTF-8) files report byte counts instead of a diff body.

## Examples

```sh
quay diff csv-parse
quay diff csv-parse --remote team-hub
quay diff csv-parse --json | jq '.files[] | select(.change != "same") | .path'
```

## Flags

| Flag | Effect |
|---|---|
| `--remote <name>` | Hub to compare against. Defaults to the default remote; with several remotes and no default, this is required rather than guessed. |
| `--profile`, `--user-config`, `--project`, `--json` | Standard globals. |

## When to use this vs …

- [`quay outdated`](outdated.md) — scans every installed skill and says which ones differ. Use it first; use `diff` to drill into one.
- [`quay add`](add.md) `--force` — what you run once you have decided to take the hub's copy.
- [`quay scan`](scan.md) — local audit only, never contacts a hub.

## Caveats

- Clones the hub to read its history (partial clone where the server allows it), so it needs network access and is slower than `outdated`.
- The history walk is capped at 50 commits touching the skill. A copy older than that reports `direction is unknown` rather than paying for an unbounded walk.
- The skill must be installed locally **and** published by the remote; either one missing is an error, not an empty report.

## `--help`

```text
Show how a locally installed skill differs from the hub's copy

Usage: quay diff [OPTIONS] <SKILL>

Arguments:
  <SKILL>  Installed skill name

Options:
      --project <PROJECT>          Project root (defaults to current directory)
      --remote <REMOTE>            Hub to compare against (defaults to the default remote)
      --user-config <USER_CONFIG>  Override user config path (defaults to ~/.config/quay/config.toml)
      --profile <PROFILE>          Override the active profile for this invocation
      --json                       Output JSON instead of human-readable text
  -h, --help                       Print help
```
