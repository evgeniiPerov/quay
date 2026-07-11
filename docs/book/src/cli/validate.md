# `quay validate`

Validate a local skill's frontmatter offline.

## Usage

```text
quay validate [OPTIONS] <SKILL>
```

Reads `.agents/skills/<SKILL>/SKILL.md` and checks:

- Frontmatter parses as YAML.
- Required fields (`name`, `description`) present.
- `name` matches the directory.
- `version` is semver-valid (if present).
- `tags` is a string array (if present).

No network calls.

## Examples

```sh
quay validate hello
quay validate hello --strict
quay validate hello --json
```

## Flags

| Flag | Effect |
|---|---|
| `--strict` | Treat missing/required-field issues as errors (exit 1). Default: warns to stderr, exits 0. |
| `--profile`, `--user-config`, `--project`, `--json` | Standard globals. |

## When to use this vs …

- Run **before** [`quay push`](push.md) — catches mistakes without paying the hub-clone cost.
- [`quay scan`](scan.md) — gives status per skill but does not deep-validate frontmatter.

## Caveats

- Default mode is **soft**: even a missing frontmatter block exits 0 with a warning. `--strict` is what you want in CI.
- SlashCommand and Freestyle skills produce a warning (no frontmatter) by design; `--strict` upgrades that to an error.
- Validation is purely structural. It does not check whether the *content* of the skill makes sense for any specific agent runtime.

## `--help`

```text
Validate a local skill's frontmatter (offline, no network)

Usage: quay validate [OPTIONS] <SKILL>

Arguments:
  <SKILL>

Options:
      --project <PROJECT>          Project root (defaults to current directory)
      --strict                     Treat missing frontmatter or required fields as errors (exit 1). Default: prints warnings to stderr, exits 0 (lenient/soft mode)
      --user-config <USER_CONFIG>  Override user config path (defaults to ~/.config/quay/config.toml)
      --profile <PROFILE>          Override the active profile for this invocation
      --json                       Output JSON instead of human-readable text
  -h, --help                       Print help
```
