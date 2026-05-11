# Introduction

**quay** is a CLI and TUI for sharing **skills** — small Markdown + YAML packages that teach AI coding agents project conventions, internal APIs, and team workflows — across many repos and many machines without a hosted registry.

Skills live as plain files in your repo at `.agents/skills/<name>/SKILL.md`. quay synchronises them with a **hub**: any git repository you control (GitHub, GitLab, Azure DevOps, Bitbucket). Push a skill to the hub; teammates `quay add <name>` it from a different project on a different machine. Mirror directories (`.claude/skills/`, `.cursor/rules/`, etc.) stay in sync automatically.

## Why not a registry-as-a-service?

- **No third-party server.** Your hub is a git repo you already trust. No new accounts, no rate limits, no vendor lock-in.
- **Audit-friendly.** Every push is a commit. Every install reads a `registry.json` committed in plain text. `git log` is your history.
- **Air-gap-friendly.** Self-host the hub on Gitea, GitLab CE, or a Bitbucket server. quay does not phone home.
- **Filesystem-first.** Skills are files. Edit them with `$EDITOR`. `git diff` works. No special API.

## Who this is for

- Engineers who run [Claude Code](https://claude.com/claude-code), [Cursor](https://cursor.com), or other agentic tools and want skill libraries that travel with the team — not the laptop.
- Teams running a mixed-provider stack (work GitHub + personal GitLab + customer Azure DevOps) who need profile-scoped configuration.
- Tooling authors building project bootstrapping flows (CI runners, devcontainers, ephemeral envs) that need declarative skill installs.

## What's next

- [Getting started](getting-started.md) — install, first profile, first push, first install. ~5 minutes.
- [Concepts](concepts.md) — the small vocabulary you need before reading the rest of the book.
- [Tutorials](tutorials/first-skill.md) — narrative walkthroughs.
- [CLI reference](cli/overview.md) and [TUI reference](tui/overview.md) — per-command, per-screen detail.
