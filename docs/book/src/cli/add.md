# `quay add`

Install a skill from a configured remote.

## Usage

```text
quay add [OPTIONS] [SKILL]
```

`SKILL` is the skill name as stored in the hub's `registry.json` (typically the directory name under `skills/`). Omit it with `-i` to pick interactively.

## Examples

```sh
quay add hello                      # install one
quay add hello world fmt-fixer      # install several
quay add -i                         # interactive checkbox picker
quay add hello --remote work        # explicit remote
quay add hello --force              # overwrite local edits
```

Inside a TTY, omitting `SKILL` and not passing `-i` triggers the picker automatically (default-interactive). Pipe `< /dev/null` to keep the prompt suppressed in scripts.

## Flags

| Flag | Effect |
|---|---|
| `--remote <NAME>` | Use a specific remote instead of the profile default. |
| `--force` | Overwrite the skill even if it already exists locally. |
| `-i, --interactive` | Open the checkbox picker. Mutually exclusive with `SKILL`. |
| `--profile`, `--user-config`, `--project`, `--json` | Standard globals. |

## When to use this vs …

- [`quay update`](update.md) — refresh an *already installed* skill to the latest hub version. `add` errors with "already installed" unless `--force`.
- [`quay info`](info.md) — preview metadata without writing files.

## Caveats

- A skill that fails `quay validate` on the hub side will still install, but you'll see warnings.
- Mirror dirs (`.claude/skills/`, `.cursor/rules/`, …) are populated automatically based on your profile's `[install].mirrors`.
- `--force` overwrites every file the fetched version contains, without confirmation — local edits to `SKILL.md` are lost. Files it *doesn't* contain are a separate question; see below. Use `quay outdated` first to see drift status.

## `--force` and local files

`--force` overwrites an existing install. Files it finds that the fetched version
does not contain are handled exactly as on `quay update` — you are asked, and
nothing is deleted unattended. `--keep-extra` and `--delete-extra` work here too.
See [update](./update.md#local-files-the-new-version-doesnt-have).

## `--help`

```text
Install a skill from a configured remote

Usage: quay add [OPTIONS] [SKILL]

Arguments:
  [SKILL]  Skill name(s) to install. Omit when using --interactive (-i)

Options:
      --project <PROJECT>          Project root (defaults to current directory)
      --remote <REMOTE>            
      --force                      Overwrite the skill if it already exists locally
      --user-config <USER_CONFIG>  Override user config path (defaults to ~/.config/quay/config.toml)
  -i, --interactive                Open an interactive checkbox list to pick skills to install. Mutually exclusive with the positional skill argument
      --profile <PROFILE>          Override the active profile for this invocation
      --json                       Output JSON instead of human-readable text
      --no-diff                    Suppress the diff body on a collision (still prints the verdict line)
      --keep-extra                 Keep local files the new version does not contain (the default when there is no terminal). Only meaningful with --force
      --delete-extra               Delete local files the new version does not contain. Only meaningful with --force
  -h, --help                       Print help
```
