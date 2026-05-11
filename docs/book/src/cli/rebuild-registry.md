# `quay rebuild-registry`

Regenerate a hub's `registry.json` from disk truth.

## Usage

```text
quay rebuild-registry [OPTIONS] [REMOTE]
```

Clones the remote, walks `skills/<name>/SKILL.md`, builds a fresh `{ skills: { <name>: { … } } }` map, commits it, and pushes per the remote's `push_mode`. `[REMOTE]` defaults to the profile's default remote when omitted.

## Examples

```sh
quay rebuild-registry                       # default remote
quay rebuild-registry work
quay rebuild-registry work --push-mode pr   # force PR even if remote says direct
```

## Flags

| Flag | Effect |
|---|---|
| `--push-mode <pr\|direct>` | Override the remote's `push_mode`. |
| `--profile`, `--user-config`, `--project`, `--json` | Standard globals. |

## When to use this vs …

- **Recovery** — `quay search` / `quay add` return "skill not found" but the file exists on the hub. The registry has drifted; run `rebuild-registry`.
- **Migration** — you imported skills into the hub manually (not via `quay push`). Rebuild fixes the registry in one shot.
- **Bad seed** — a hub started with a malformed `registry.json` (e.g. `{ skills: [] }` array instead of map). Rebuild regenerates the correct shape.
- [`quay push`](push.md) — already updates the registry on every push. Reach for `rebuild-registry` only when the registry is wrong.

## Caveats

- Rewrites `registry.json` from scratch. Any hand-edits there are lost.
- The commit message is a stock "rebuild registry" message; if you want a custom one, edit before the PR is merged.
- Works against hubs with no `registry.json` at all (creates one).

## `--help`

```text
Rebuild a hub's `registry.json` from disk truth and push it back.

Clones the remote, walks `skills/<name>/SKILL.md`, regenerates `registry.json` containing every discovered skill, then commits and pushes (PR or direct per the remote's `push_mode`).

Usage: quay rebuild-registry [OPTIONS] [REMOTE]

Arguments:
  [REMOTE]
          Remote name. Uses the default remote when omitted

Options:
      --project <PROJECT>
          Project root (defaults to current directory)

      --push-mode <PUSH_MODE>
          Override the remote's `push_mode` for this invocation
          [possible values: pr, direct]

      --user-config <USER_CONFIG>
          Override user config path (defaults to ~/.config/quay/config.toml)

      --profile <PROFILE>
          Override the active profile for this invocation

      --json
          Output JSON instead of human-readable text

  -h, --help
          Print help (see a summary with '-h')
```
