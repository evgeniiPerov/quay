# `quay init`

Initialise quay in the current project.

## Usage

```text
quay init [OPTIONS]
```

Creates a `.quay/` directory with an empty `config.toml`. The project config is layered on top of `~/.config/quay/config.toml` — anything you set here overrides the user-level value for this project only.

## Examples

```sh
quay init                           # current dir
quay init --project ~/code/foo      # explicit
```

Confirm:

```sh
ls .quay/
# config.toml
```

## Flags

| Flag | Effect |
|---|---|
| `--project <PATH>` | Project root (default: cwd). |
| `--user-config <PATH>` | Override user config path. |
| `--profile <NAME>` | Override active profile for this invocation. |
| `--json` | JSON output. |

## When to use this vs …

- Use `quay init` once per repo if you want a per-project override (different mirror dirs, custom remote pinned to that project).
- If your user-level config already covers everything, `init` is optional — `add` / `push` work fine without it.

## Caveats

- Re-running `init` on a project that already has `.quay/config.toml` leaves the existing file untouched. There is no `--force`.
- Commit `.quay/config.toml` so teammates inherit the project config. `.quay/push-log.json` is gitignored (local-only intent log).

## `--help`

```text
Initialize quay in the current project

Usage: quay init [OPTIONS]

Options:
      --project <PROJECT>          Project root (defaults to current directory)
      --user-config <USER_CONFIG>  Override user config path (defaults to ~/.config/quay/config.toml)
      --profile <PROFILE>          Override the active profile for this invocation
      --json                       Output JSON instead of human-readable text
  -h, --help                       Print help
```
