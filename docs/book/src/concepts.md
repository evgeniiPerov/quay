# Concepts

Short definitions for the vocabulary used in the rest of the book.

## Skill

A directory under `.agents/skills/<name>/` containing at least `SKILL.md`. The Markdown body is what an AI agent reads; the optional YAML frontmatter (`name`, `description`, `version`, `tags`) gives quay structured metadata.

quay tolerates three skill formats:

- **Frontmatter** — YAML block at the top. Versioned, tagged, the recommended form.
- **SlashCommand** — single-line H1 in the form `# /<name>`. Convention for slash-command-style skills.
- **Freestyle** — anything else. Treated as a skill with defaults (dir-name = name, version = `0.0.0`).

## Hub

Any git repository you control. quay reads and writes `skills/<name>/SKILL.md` plus a top-level `registry.json` describing the set. A hub is just a repo — clone it, browse it on GitHub, run CI against it.

## Profile

A named bundle of defaults in `~/.config/quay/config.toml`: the default hub URL, branch, `push_mode`, mirror dirs (`.claude/skills/`, `.cursor/rules/`, …), and a `provider` hint. Switch with `quay profile use -i`.

## Remote

A profile's hub URL plus its provider kind (one of `github`, `githubenterprise`, `gitlab`, `bitbucket`, `azuredevops`). Per-command override: `quay push --remote work-gitlab …`.

## Provider

The plug-in that knows how to talk to a specific git host: how to parse the URL, how to open a PR (`gh`, `glab`, `az`), how to construct a fallback compare URL, and how to run a connectivity check. Bitbucket has no CLI binding; quay always falls back to a compare URL there.

## Mirror dirs

Tools like Claude Code read skills from `.claude/skills/`, Cursor from `.cursor/rules/`. quay keeps copies of installed skills synced from the canonical `.agents/skills/` into these mirror dirs based on each profile's `mirrors` setting.

## Registry

`registry.json` at the hub root. A `{ skills: { <name>: { version, … } } }` map describing the hub's contents. `quay search` and `quay add` read it. `quay rebuild-registry <remote>` regenerates it from `skills/*/SKILL.md` if it ever drifts.

## Push log

`~/.config/quay/push-log.json`. An append-only record of every successful push: skill name, remote, commit SHA or PR URL, timestamp, source project path. Drives the Dashboard "recent pushes" panel and `pushed-local` / `pushed-direct` status badges.

## Drift detection

`quay outdated` and `quay scan` compute a SHA-on-the-fly for every installed skill and compare to the hub's recorded SHA. A modified local file gets the `installed-modified` badge; the hub having a newer commit gets `outdated`. `quay add --force` overwrites local drift.

## `push_mode`

Per-remote setting controlling commit behaviour:

- **`pr`** (default) — quay creates a feature branch, pushes it, and opens a PR via the provider CLI (`gh`, `glab`, `az`). Bitbucket falls back to a compare URL.
- **`direct`** — quay commits straight to the default branch and `git push`. Used for repos with no branch protection where review overhead is not worth it.

Override per-call: `quay push --push-mode direct`.

## Default-interactive

Most CLI commands detect a TTY and prompt for missing arguments. Pipe them into `xargs` / scripts and they switch to non-interactive: `quay push hello < /dev/null` errors instead of prompting. The `[i]` indicator in `--help` output marks flags that have wizard equivalents.
