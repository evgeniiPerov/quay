# quay

**Git-native skill registry for AI coding agents.** Share `SKILL.md` files (Claude Code, Codex, Cursor, …) between developers, teammates, and orgs — using any git repo as the hub. No SaaS, no tokens beyond `git`, no lock-in.

```text
$ quay search csv
csv-parse        Parse CSV with type inference.
$ quay add csv-parse
installed csv-parse@0.3.1 → .agents/skills/csv-parse/
$ quay push my-skill --bump=patch
opened PR: https://github.com/acme/skills/pull/142
```

## Why?

Agents are eating dev workflows, but the **skills** that make them useful (prompts, validators, scaffolds, retry policies) get copy-pasted from project to project, drift, and die in private gists.

Quay treats skills like packages:

- **Author once.** Drop a `SKILL.md` into `.agents/skills/<name>/` in any project (or use `quay create`).
- **Push to a hub.** Hub = a regular git repo your team owns. Push opens a PR.
- **Pull anywhere.** `quay add <skill>` installs into `.agents/skills/`, lockfile pins the sha, mirrors propagate to `.claude/skills/`, `.codex/skills/`, etc.
- **Stay in git.** Permissions = repo permissions. Audit = `git log`. Hosting = whatever you already use.

Works for: a Next.js team sharing a `react-perf-review` skill, an org standardising a `pr-description` skill across services, a contractor publishing skills to clients via private GitHub repos.

## Status

| Plan | Scope | State |
|---|---|---|
| 1 / 1.5 | foundation, read-only CLI | ✅ |
| 2 / 2.5 | search, outdated, update, sync | ✅ |
| 3 | create, validate, push (PR-based) | ✅ |
| 4 | profiles (multi-org) | ✅ |
| 5 | mirroring (`quay link`) | ✅ |
| 6 / 6.5 / 6.75 / 6.85 | TUI MVP, settings, create/push, paste-friendly forms | ✅ |
| 7a | provider abstraction (GitHub / GHE / GitLab / Bitbucket / Azure DevOps) + live `remote test` | ✅ |
| 8 | scan-first flow, format-tolerant push, mixed-format skills | ✅ |
| 7b | packaged releases (cargo-dist, Homebrew tap) | planned |

**287 tests, 0 clippy warnings, MSRV `stable`.** Plans live in [`docs/superpowers/plans/`](docs/superpowers/plans/). Active design doc: [`docs/superpowers/specs/2026-05-08-quay-cli-design.md`](docs/superpowers/specs/2026-05-08-quay-cli-design.md).

## Install

From source (until Plan 7b ships packaged releases):

```sh
git clone https://github.com/evgeniiPerov/quay.git
cd quay/apps/cli
cargo build --release
cp target/release/quay ~/.local/bin/   # or anywhere on PATH
```

Requires the `git` CLI on `PATH`. `gh` is optional — used to auto-open PRs on `quay push`; without it, push prints a compare URL for manual PR creation.

Supported hub providers: **GitHub.com**, **GitHub Enterprise**, **GitLab** (cloud + self-hosted, nested subgroups), **Bitbucket Cloud**, **Azure DevOps Services**. Auto-detected from URL; override with `quay remote add --provider <kind>`.

## Quickstart

### Author + share a skill (you)

```sh
# 1. Profile + hub (one time)
quay profile add work --email you@example.com \
  --remote main=https://github.com/your-org/skills.git
quay remote test main

# 2. In any project (Next.js, Rust, whatever)
cd ~/code/your-app
quay init
quay create http-retries
$EDITOR .agents/skills/http-retries/SKILL.md
quay validate http-retries
quay push http-retries --bump=patch        # opens PR
```

Already have skills lying around in `.agents/skills/`? Skip `create`:

```sh
quay scan                 # discovers existing skills, any format
quay push <name>          # push as-is, frontmatter optional
```

### Pull a teammate's skill (colleague)

```sh
cd ~/code/their-app
quay init
quay remote add team https://github.com/your-org/skills.git --default
quay search retries
quay add http-retries
```

### Stay in sync

```sh
quay outdated             # show drift
quay update               # bump everything
quay sync                 # rehydrate from lockfile (CI / fresh clone)
```

### TUI

```sh
quay tui                  # full-screen browse / search / install / push
```

Onboarding wizard runs on first launch when no profile is configured.

See [`apps/cli/README.md`](apps/cli/README.md) for the full command list and `--json` output examples.

## Concepts

- **Hub.** A git repo with `skills/<name>/SKILL.md` files plus a `registry.json` index. Any provider above works.
- **Profile.** A named bundle of `(identity + remotes)`. One profile per org; switch with `quay profile use <name>` or override per command via `--profile=<name>` / `QUAY_PROFILE`.
- **Skill.** A Markdown file with YAML frontmatter (`name`, `description`, `version`, `tags`, …). Lives at `.agents/skills/<name>/SKILL.md` after install. Tool-specific mirrors (`.claude/skills/`, `.codex/skills/`, …) handled by `quay link`.
- **Lockfile.** `.agents/skills.lock.json` pins each installed skill to a content sha. `quay sync` reproduces an exact tree from it; commit the file.
- **Scan.** `quay scan` walks `.agents/skills/` and labels each entry as `Local`, `Installed`, `InstalledModified`, or `PushedLocal` — lets you adopt existing skill folders without rewriting them.

## Repo layout

```
apps/cli/                  Rust workspace
├── crates/quay-core/      domain logic (config, registry, fetcher, manager,
│                          scanner, providers/{github,gitlab,bitbucket,azure})
├── crates/quay-cli/       clap subcommands + ratatui TUI
└── crates/quay/           binary
docs/superpowers/
├── specs/                 design docs
└── plans/                 implementation plans (used by Claude Code agents)
.agents/                   shared agent rules + skills (tool-neutral)
.claude/                   Claude Code-specific configuration
```

## Contributing

PRs welcome. Run before opening:

```sh
cd apps/cli
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test
```

The codebase is implemented plan-by-plan via subagent-driven development; see [`.agents/rules/`](.agents/rules/) for code-style and testing conventions agents follow.

## License

[MIT](LICENSE).
