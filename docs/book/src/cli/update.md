# `quay update`

Update installed skills to the latest hub version.

## Usage

```text
quay update [OPTIONS] [SKILL]
```

Omit `SKILL` to update every installed skill that has a newer version on the hub. Inside a TTY this auto-opens a checkbox picker of outdated skills; pass `--all` to bypass the picker, or `--dry-run` to preview.

## Examples

```sh
quay update                          # TTY: opens picker of outdated. Non-TTY: updates all.
quay update hello                    # just this one
quay update --all                    # all outdated, no picker
quay update --dry-run                # show what would change
quay update -i                       # explicit picker
```

## Flags

| Flag | Effect |
|---|---|
| `--dry-run` | Show what would change without writing. |
| `-i, --interactive` | Open the picker. |
| `--all` | Skip the TTY auto-picker; update everything outdated. |
| `--keep-extra` | Keep local files the new version does not contain, without prompting or printing the note. |
| `--delete-extra` | Delete local files the new version does not contain, without prompting. |
| `--profile`, `--user-config`, `--project`, `--json` | Standard globals. |

## When to use this vs …

- [`quay outdated`](outdated.md) — list-only sibling; identical detection logic without writing anything.
- [`quay add`](add.md) `<skill> --force` — overwrites local drift even when versions match.

## Caveats

- "Outdated" uses semver comparison on the frontmatter `version` field. Skills without frontmatter (SlashCommand / Freestyle) never show as outdated by version — they're compared by SHA-on-fly instead.
- Local edits are detected and surfaced: an outdated skill with local modifications is reported as `installed-modified`. `update` refuses to overwrite it without `--force` (which lives on `quay add`, not `update` — use `quay add <name> --force` to nuke).
- `--dry-run` does not query CLI tools (`gh` / `glab`); pure diff against hub clone.

## Local files the new version doesn't have

A skill is a directory, and yours may hold files the hub's copy does not — notes
you wrote, or a file the hub has since deleted. quay cannot tell those two apart:
the lockfile records a content hash, not a file list, so nothing on disk says what
the previous install put there.

So it asks. When an update finds files the new version does not contain, it lists
them and offers to keep them, delete them, pick individually, or keep these and
every remaining skill's without asking again.

```text
csv-parse: 2 files not in the new version
  refs/legacy.md
  notes.md
```

Without a terminal — CI, a pipe, or `--json` — nothing is deleted. quay keeps the
files and says so:

```text
note: csv-parse — kept 2 files not in the new version
      (refs/legacy.md, notes.md). Pass --delete-extra to remove them.
```

| Flag | Effect |
|---|---|
| `--keep-extra` | Keep them, no prompt, and no note either — this is the only way to suppress it. |
| `--delete-extra` | Delete them, no prompt. Works without a terminal. |

Note that the bare unattended default (no flag, no terminal) also keeps everything —
`--keep-extra` only changes whether the note is printed.

Dotfiles, dot-directories and symlinks are never offered and never deleted. They
are outside the set quay manages, and one of them — `.quay-mirror` — is what marks
a copy mirror as quay-managed.

The "not in the new version" set is derived from the hub's `registry.json`
`files` list for that skill, not from the actual fetched tree. A hub whose
registry has gone stale — edited by hand without a `quay rebuild-registry` —
can therefore list a file as missing when it still exists upstream. An
interactive prompt shows the filename, so a human can recognize and keep it;
`--delete-extra` in an unattended run has no such backstop and will delete it.

## `--help`

```text
Update installed skills to the latest available version

Usage: quay update [OPTIONS] [SKILL]

Arguments:
  [SKILL]  Update only this skill; if omitted, updates every installed skill

Options:
      --dry-run                    Show what would change without writing to disk
      --project <PROJECT>          Project root (defaults to current directory)
  -i, --interactive                Open an interactive checkbox list of outdated skills to update. Mutually exclusive with the positional skill argument
      --user-config <USER_CONFIG>  Override user config path (defaults to ~/.config/quay/config.toml)
      --all                        Update every installed skill without opening the picker, even in a terminal. Explicit bypass for the TTY auto-trigger
      --profile <PROFILE>          Override the active profile for this invocation
      --json                       Output JSON instead of human-readable text
      --keep-extra                 Keep local files the new version does not contain (the default when there is no terminal)
      --delete-extra               Delete local files the new version does not contain
  -h, --help                       Print help
```
