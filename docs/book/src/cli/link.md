# `quay link`

Manage mirror dirs declared under `[install].mirrors` in the active profile.

## Usage

```text
quay link [OPTIONS] [SUBCOMMAND]
```

Subcommands:

| Subcommand | What |
|---|---|
| `check` | Verify mirrors are intact; exit non-zero if drifted. |
| `add <PATH>` | Add a mirror to the project config and populate it for every installed skill. |
| `remove <PATH>` | Remove a mirror from the project config (does not delete files). |

With no subcommand, `quay link` applies the configured mirrors for the project (idempotent).

## Examples

```sh
quay link                              # apply mirrors
quay link check                        # CI: fail if mirrors drifted
quay link add .cursor/rules            # add a Cursor mirror
quay link add .claude/skills --strategy auto
quay link remove .cursor/rules         # forget the mirror (files stay)
```

## Flags

| Flag | Effect |
|---|---|
| `--strategy <NAME>` (on `add`) | Mirror strategy. Default `auto` (let quay pick per skill format). |
| `--force` | Overwrite existing entries even if they conflict with quay's layout. |
| `--profile`, `--user-config`, `--project`, `--json` | Standard globals. |

## When to use this vs …

- [`quay add`](add.md) — already applies your current mirrors. `link` is for *changing* the mirror set or auditing it.
- [`quay scan`](scan.md) — does not check mirror drift; it checks skill drift. Pair `scan` + `link check` for a full audit.

## Caveats

- `link remove` deletes the *config entry*, not the directory on disk. To also clean files, `rm -rf` afterwards.
- `link check` is the canonical CI gate. Exit code 0 = clean; non-zero = at least one mirror is missing or out-of-sync.
- `--force` will overwrite hand-edited mirror dirs without backup. Use carefully.

## `--help`

```text
Apply or verify mirrors from `[install].mirrors` config

Usage: quay link [OPTIONS] [COMMAND]

Commands:
  check   Verify mirrors are intact; exit non-zero if drifted
  add     Add a new mirror to the project config and apply it for every installed skill
  remove  Remove a mirror from the project config (does not delete the mirror dir)
  help    Print this message or the help of the given subcommand(s)
```
