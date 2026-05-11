# Author your first skill

End-to-end walkthrough: write a small skill, push it to a hub, install it from a different project.

Pre-reqs: `quay` on `PATH`, a profile already configured (`quay profile add -i` if not).

## 1. Pick a tiny example

We'll author a skill that teaches an agent to apply a one-rule lint fix: "use early returns instead of nested `else`."

```sh
mkdir -p ~/code/origin-project/.agents/skills/early-returns
cd ~/code/origin-project
$EDITOR .agents/skills/early-returns/SKILL.md
```

Paste:

```markdown
---
name: early-returns
description: Refactor nested `else` chains into early returns.
version: 0.1.0
tags: [refactor, code-style]
---

# Early returns

When you see code shaped like:

```js
if (cond) {
  doA();
} else {
  doB();
}
```

…and `doA` is short, prefer an early return:

```js
if (cond) {
  doA();
  return;
}
doB();
```

Apply to JavaScript, TypeScript, and Rust. Skip Python (no early-return idiom in match-case).
```

## 2. Validate offline

```sh
quay validate early-returns
```

Expect `OK` (or warnings on stderr; exit 0 in soft mode). Fix anything flagged before pushing.

## 3. Push

```sh
quay push early-returns
```

What happens:

1. `quay` clones your default-profile hub into a temp dir.
2. Copies `SKILL.md` to `skills/early-returns/SKILL.md`.
3. Updates the hub's `registry.json` with name, description, version, tags.
4. Commits.
5. Per your remote's `push_mode`: opens a PR (default) or pushes the branch directly.
6. Prints the PR URL or commit SHA.
7. Appends an entry to `~/.config/quay/push-log.json`.

If `push_mode = pr`, merge the PR before continuing.

## 4. Consume from a different project

In a fresh shell, fresh directory:

```sh
mkdir -p ~/code/consumer-project
cd ~/code/consumer-project
git init      # optional; quay does not require git
quay add early-returns
```

The skill lands at `.agents/skills/early-returns/SKILL.md`. Mirror dirs (`.claude/`, `.cursor/`, …) get copies based on your profile's `[install].mirrors`.

Confirm:

```sh
quay list
quay scan
```

`scan` should show `early-returns` with status `installed`.

## 5. What if you edit the local copy?

Edit `consumer-project/.agents/skills/early-returns/SKILL.md` — add a sentence. Re-scan:

```sh
quay scan
```

Status flips to `installed-modified`. `quay outdated` will refuse to overwrite it. To force re-sync:

```sh
quay add early-returns --force
```

## Next

- [Multi-provider setup](multi-provider.md) — same flow, but a second hub on a different provider.
- [`quay push`](../cli/push.md) — full flag reference, including `--bump`.
