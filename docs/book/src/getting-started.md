# Getting started

Five minutes from zero to a skill shared between two projects.

## 1. Install

Homebrew:

```sh
brew install evgeniiPerov/tap/quay
```

Or download a release binary from <https://github.com/evgeniiPerov/quay/releases> and put it on your `PATH`.

Verify:

```sh
quay --version
```

> **macOS Gatekeeper:** the first run may be blocked. Right-click the binary → **Open** once, or `xattr -dr com.apple.quarantine /opt/homebrew/bin/quay`.

## 2. Create a profile

A **profile** holds your default hub URL, default branch, push mode (PR or direct), and provider kind. The interactive wizard sets everything in one go:

```sh
quay profile add -i
```

Pick a name (e.g. `work`), paste your hub URL (e.g. `git@github.com:my-org/skills-hub.git`), answer two more prompts. Done.

Confirm it took:

```sh
quay profile show
```

## 3. Author a skill

From any project directory:

```sh
mkdir -p .agents/skills/hello
$EDITOR .agents/skills/hello/SKILL.md
```

Paste a minimal frontmatter skill:

```markdown
---
name: hello
description: Say hello.
version: 0.1.0
---

# Hello

Print "hi" when the agent starts.
```

## 4. Push

```sh
quay push hello
```

quay clones your hub into a temp dir, drops the skill into `skills/hello/SKILL.md`, updates the hub's `registry.json`, commits, and (depending on `push_mode`) opens a PR or pushes the commit directly. The URL is printed on success.

## 5. Install in a different project

In a freshly-cloned repo on another machine:

```sh
quay add hello
```

The skill lands at `.agents/skills/hello/SKILL.md`. Mirror dirs (`.claude/`, `.cursor/`, etc.) are populated automatically based on your profile.

## Next steps

- [Author your first skill](tutorials/first-skill.md) — same flow, with the why behind each step.
- [Multi-provider setup](tutorials/multi-provider.md) — run work GitHub + personal GitLab on one machine.
- [Concepts](concepts.md) — the terms used in the rest of the book.
