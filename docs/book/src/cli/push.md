# `quay push`

Publish a local skill to a hub.

## Usage

```text
quay push [OPTIONS] [SKILL]
```

`quay push` clones the configured hub into a temp dir, drops your skill at `skills/<name>/SKILL.md`, updates the hub's `registry.json`, commits, and then either opens a PR (default) or pushes to the default branch directly (`--push-mode direct` or per-remote setting).

## Examples

```sh
quay push hello                              # default profile, default remote, push_mode from remote config
quay push hello --remote work                # explicit remote
quay push hello --push-mode direct           # bypass remote setting, push straight to main
quay push hello --bump patch                 # 0.1.0 → 0.1.1 before pushing
quay push hello --bump minor
quay push hello --bump major
quay push -i                                 # interactive checkbox picker
```

After success, the PR URL (or commit SHA, for direct mode) is printed. The push is also logged to `~/.config/quay/push-log.json`.

## Flags

| Flag | Effect |
|---|---|
| `--remote <NAME>` | Target a specific remote. |
| `--bump <KIND>` | Bump version in frontmatter: `patch` / `minor` / `major` / `as-written` (default). |
| `--push-mode <pr\|direct>` | Override the remote's `push_mode` for this invocation. |
| `-i, --interactive` | Open the checkbox picker. |
| `--profile`, `--user-config`, `--project`, `--json` | Standard globals. |

## When to use this vs …

- [`quay rebuild-registry`](rebuild-registry.md) — regenerate a corrupted/missing `registry.json`. Use after manual hub surgery, not as a normal publish step.
- TUI **Local** screen + `[u]` (single) / `[U]` (bulk selected via `[Space]`) — same flow with a UI.
- [`quay validate`](validate.md) first — catches frontmatter mistakes offline before incurring a hub clone.

## Caveats

- `--bump` requires Frontmatter format. SlashCommand / Freestyle skills get a clean error if you pass `--bump` against them.
- Direct mode against a branch-protected default branch will fail at `git push origin main`. The error hint suggests `--push-mode pr` or setting `push_mode = "pr"` in the remote config.
- The first push to an empty hub seeds `registry.json` automatically — no manual setup needed.
- Bitbucket has no CLI client. PR mode falls back to printing a compare URL; you must open the PR in the browser manually.

## `--help`

```text
Push a local skill to a hub via PR (or directly, if --push-mode=direct or the remote's TOML says so)

Usage: quay push [OPTIONS] [SKILL]

Arguments:
  [SKILL]  Skill name to push. Omit when using --interactive (-i)

Options:
      --project <PROJECT>          Project root (defaults to current directory)
      --remote <REMOTE>
      --bump <BUMP>                Bump kind: patch | minor | major | as-written. Default: as-written [default: as-written]
      --user-config <USER_CONFIG>  Override user config path (defaults to ~/.config/quay/config.toml)
      --profile <PROFILE>          Override the active profile for this invocation
      --push-mode <PUSH_MODE>      Override the remote's `push_mode` setting for this invocation. Values: pr, direct [possible values: pr, direct]
  -i, --interactive                Open an interactive checkbox list of local skills to push. Mutually exclusive with the positional skill argument
      --json                       Output JSON instead of human-readable text
  -h, --help                       Print help
```
