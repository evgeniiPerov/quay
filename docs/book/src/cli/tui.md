# `quay tui`

Launch the interactive terminal UI.

## Usage

```text
quay tui [OPTIONS]
```

Boots the ratatui-based interface. On first run with no profiles, the **Onboarding** screen runs through profile + remote setup; afterwards every launch starts on **Dashboard**.

## Examples

```sh
quay tui                                # default
quay tui --profile work                 # launch under specific profile
quay tui --project ~/code/foo
```

## Flags

| Flag | Effect |
|---|---|
| `--profile`, `--user-config`, `--project`, `--json` | Standard globals. `--json` is ignored by the TUI itself; the global is still parseable. |

## What you can do

| Key | Action |
|---|---|
| `[1]` | Dashboard |
| `[2]` | Local skills |
| `[3]` | Remote skills |
| `[s]` | Search |
| `[,]` | Settings |
| `[p]` | Profile switcher modal |
| `[g] [c]` | Create / push (chord) |
| `[q]` | Quit |

See the [TUI overview](../tui/overview.md) for per-screen detail.

## When to use this vs …

- CLI subcommands — when scripting / piping output / CI. The TUI is for humans.
- [`quay add`](add.md) `-i` / [`quay push`](push.md) `-i` — quick one-shot pickers in CLI. The TUI gives you the same flow plus context (status, history, previews).

## Caveats

- The TUI requires a real TTY. Piping its output (`quay tui | tee`) is unsupported.
- Bracketed paste is enabled in setup; if your terminal lacks support, pasting still works but multi-line paste may register as separate keystrokes.
- `xdg-open` / `open` / `cmd-start` must be on `PATH` for the "open in browser" affordance to work (e.g. after a PR is created from Create/Push).

## `--help`

```text
Launch the interactive TUI

Usage: quay tui [OPTIONS]

Options:
      --project <PROJECT>          Project root (defaults to current directory)
      --user-config <USER_CONFIG>  Override user config path (defaults to ~/.config/quay/config.toml)
      --profile <PROFILE>          Override the active profile for this invocation
      --json                       Output JSON instead of human-readable text
  -h, --help                       Print help
```
