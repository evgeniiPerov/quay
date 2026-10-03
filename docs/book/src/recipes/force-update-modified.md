# Force update over local edits

You tweaked an installed skill, the hub has since moved on, and you want the
hub's copy back — or you aren't sure what changed and want to look before you
lose anything. quay never resolves this silently: a plain `quay add` on a skill
you already have shows you the difference and stops.

## Look first

```sh
quay scan          # status column: installed-modified = edited since install
quay outdated      # version bumps, and content that differs whatever the version says
quay diff csv-parse
```

`quay diff` compares the whole skill directory and gives a verdict from hub
history — hub ahead, your copy ahead, or changed in an unknown direction — then
a per-file diff. `+` lines are what the hub would give you, `-` lines are what
you'd lose. It writes nothing.

If any of your edits are worth keeping, push them first
(`quay push csv-parse`) or copy them somewhere. Nothing below has an undo
except your project's own git history.

## Take the hub's copy

```sh
quay add csv-parse --force
```

This overwrites every file the hub's version contains. It skips the
reconcile step entirely — no verdict, no prompt.

Without `--force`, in a terminal, `quay add csv-parse` prints the verdict and
diff and asks what to do with that one skill. Use that when you want to decide
per skill rather than overwrite.

## Files only you have

The hub's version may not contain every file in your copy — notes you added, or
a file the hub deleted since. quay can't tell those apart, so in a terminal it
lists them and asks. Decide up front instead:

```sh
quay add csv-parse --force --keep-extra     # keep them, no prompt
quay add csv-parse --force --delete-extra   # exact hub copy, no prompt
```

With no terminal and neither flag, they are kept. Dotfiles and symlinks are
never touched.

## Several at once

```sh
quay add -i --force
```

In the picker, `--force` overwrites every selected skill. Without it, quay asks
once for all collisions: update all, skip all, or prompt per skill.

## Gotchas

- **`quay update` won't help with drift — for frontmatter skills.** It acts on
  version upgrades there. A skill whose content differs at the same version —
  your edit, or a hub fix that didn't bump `version` — needs `quay add --force`.
  Slash-command and freestyle skills (no YAML frontmatter) are compared by
  content, so for them `update` does act on drift — and overwrites your edits.
  A frontmatter skill that merely omits `version` is not: it counts as `0.0.0`
  on both sides, so `update` skips it — use `quay add --force`.
- **Mirrors follow.** Configured mirrors (`[install].mirrors`) are re-applied
  after the install, so `.claude/skills/csv-parse` and friends get the new copy
  too.
- **`--delete-extra` trusts the hub's `registry.json`.** If the registry is stale,
  a file that still exists upstream can be deleted. `quay rebuild-registry` on
  the hub fixes the registry.
