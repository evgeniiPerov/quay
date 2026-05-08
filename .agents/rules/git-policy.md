---
name: git-policy
description: Git policy that applies to every assistant in every directory of this repo. No path scope.
---

# Git Policy (repo-wide)

Applies to **every** AI assistant working in this repository. No path scope — universal.

## Hard rules

- **Assistants do not run `git commit`.** The user commits. If you've finished a logical chunk, summarize the diff and stop.
- **Assistants do not run `git push`, `git push --force`, or `git push --force-with-lease`.** Ever.
- **Assistants do not modify `.git/config` or shell out to `git config`.**
- **Assistants do not run destructive operations without explicit per-invocation user approval:** `git reset --hard`, `git checkout --`, `git restore .`, `git clean -f`, `git branch -D`, `git rebase -i`, `git stash drop`, `git filter-branch`, `git replace`.
- **Assistants do not skip hooks** (`--no-verify`, `--no-gpg-sign`) unless the user has explicitly asked in the same turn.
- **Assistants do not amend commits** unless the user explicitly says "amend". Pre-commit hook failures do **not** authorize an amend — fix the issue and create a NEW commit.

## Allowed without prompting

- Read-only: `git status`, `git diff`, `git log`, `git show`, `git blame`, `git branch -v`, `git remote -v`.
- Branch creation: `git checkout -b <name>`, `git switch -c <name>`.
- Staging: `git add <specific paths>`. **Never** `git add -A` or `git add .` — too easy to stage `.env`, credentials, generated artifacts.

## Branch hygiene

- Feature branches: `<type>/<short-slug>` — e.g. `feat/quay-init`, `fix/lockfile-dedupe`, `chore/agents-readme`.
- Don't rename `main`. Don't push to `main` directly.

## When in doubt

Stop and ask. The cost of a confirmation prompt is low; the cost of a force-push to `main` is not.

## Why this is universal

Quay is a CLI for sharing skills across orgs. Skills run with full agent permissions. A misbehaving skill that force-pushes is a worst-case incident. This policy is the floor; individual skills may layer stricter rules on top.
