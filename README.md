# quay

**Git-native skill registry for AI coding agents.** Share `SKILL.md` files (Claude Code, Codex, Cursor, …) between developers, teammates, and orgs — using any git repo as the hub. No SaaS, no tokens beyond `git`, no lock-in.

```text
$ quay search csv
csv-parse        Parse CSV with type inference.
$ quay add csv-parse
installed csv-parse → .agents/skills/csv-parse/
$ quay push my-skill --bump=patch
opened PR: https://github.com/acme/skills/pull/142
```

## Install

### Homebrew (macOS, Linux)

```sh
brew install evgeniiPerov/tap/quay
```

### Manual download

Grab the matching tarball or zip from
<https://github.com/evgeniiPerov/quay/releases/latest> and extract the
`quay` binary into a directory on your `PATH`.

### macOS Gatekeeper note

Binaries are unsigned in v0.1. If macOS blocks the binary on first launch:

```sh
xattr -d com.apple.quarantine /usr/local/bin/quay
```

…or right-click the binary in Finder once and choose **Open**.

## Why?

Agents are eating dev workflows, but the **skills** that make them useful (prompts, validators, scaffolds, retry policies) get copy-pasted from project to project, drift, and die in private gists.

Quay treats skills like packages:

- **Author once.** Drop a `SKILL.md` into `.agents/skills/<name>/` in any project — write it by hand, generate it with AI, or use any editor.
- **Push to a hub.** Hub = a regular git repo your team owns. Push opens a PR.
- **Pull anywhere.** `quay add <skill>` installs into `.agents/skills/`; mirrors propagate to `.claude/skills/`, `.codex/skills/`, `.cursor/skills/` via `quay link`.
- **Stay in git.** No lockfile, no lock-in. Skills tracked by git history. Permissions = repo permissions. Audit = `git log`.

Works for: a Next.js team sharing a `react-perf-review` skill, an org standardising a `pr-description` skill across services, a contractor publishing skills to clients via private GitHub repos.

Requires `git` on `PATH`. `gh` / `glab` / `az` is optional — used to auto-open PRs on `quay push`; without it, push prints a compare URL or runs in `direct` mode if the remote is configured for it.

Supported hub providers: **GitHub.com**, **GitHub Enterprise**, **GitLab** (cloud + self-hosted, nested subgroups), **Bitbucket Cloud**, **Azure DevOps Services**. Auto-detected from URL; override with `quay remote add --provider <kind>`.

## Status

| Plan | Scope | State |
|---|---|---|
| 1–5 | foundation, read-only CLI, search, create/validate/push, profiles, mirroring | ✅ |
| 6 / 6.5 / 6.75 / 6.85 | TUI MVP, settings, create/push, paste-friendly `ratatui-form` | ✅ |
| 7a | provider abstraction (GitHub / GHE / GitLab / Bitbucket / Azure DevOps) + live `remote test` | ✅ |
| 7b | packaged releases (cargo-dist, GitHub Releases, Homebrew tap, PowerShell installer) | ✅ |
| 8 | scan-first flow, format-tolerant push, mixed-format skills | ✅ |
| 9 | per-remote `push_mode` (`pr` default, `direct` opt-in via pure git, no provider CLI) | ✅ |
| 10 | filesystem-first model: drop lockfile + sync + create; multi-mirror scanner; `scan` mirrors+drift columns | ✅ **v0.2.0** |

## Quickstart

### Author + share a skill (you)

### Setting up profiles

Three ways to create a profile:

#### Interactive wizard

```sh
quay profile add -i
```

Walks through name, email, remote(s) with provider auto-detection, push mode,
default flag, and activation. Add as many remotes as you need in one session.

#### From a TOML file or stdin

```sh
quay profile add ci --from-toml ci-profile.toml
# or from stdin:
cat profile.toml | quay profile add ci --from-toml -
```

#### Explicit flags (scriptable)

```sh
quay profile add work \
  --email you@example.com \
  --remote main=git@github.com:org/skills.git --provider github --push-mode pr --default \
  --remote azure=git@ssh.dev.azure.com:v3/org/proj/repo --provider azuredevops --push-mode direct \
  --activate
```

The three modes are mutually exclusive; clap will error if combined.

```sh
# 1. Profile + hub (one time)
quay profile add work --email you@example.com \
  --remote main=https://github.com/your-org/skills.git
quay remote test main

# 2. In any project (Next.js, Rust, whatever)
cd ~/code/your-app
quay init
$EDITOR .agents/skills/http-retries/SKILL.md   # human or AI agent writes the skill
quay validate http-retries
quay push http-retries --bump=patch        # opens PR
```

Already have skills lying around in `.agents/skills/`? Just push:

```sh
quay scan                 # discovers existing skills, any format; shows mirrors + drift
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
quay outdated             # show drift (no lockfile — compares live registry vs local files)
quay update               # pull latest version of installed skills
# No `quay sync` — skills tracked by git history; commit your changes normally
```

### TUI

```sh
quay tui                  # full-screen browse / search / install / push
```

Onboarding wizard runs on first launch when no profile is configured.

#### Bulk select

Press `[Space]` in Local (screen `[2]`) or Remote (screen `[3]`) to toggle selection on a row.
With one or more rows selected:

- `[u]` / `[U]` — push every selected skill (Patch bump in `[u]`; shared form in `[U]`)
- `[a]` / `[A]` — pull every selected skill (block on collisions / force overwrite)
- `[d]` / `[D]` — delete every selected skill (locally / everywhere)
- `[Esc]` — clears the selection

The same workflow is available from the CLI:

```sh
quay push -i      # checkbox list of local skills
quay add  -i      # checkbox list of remote registry rows
quay update -i    # checkbox list of outdated skills
```

See [`apps/cli/README.md`](apps/cli/README.md) for the full command list and `--json` output examples.

## Concepts

- **Hub.** A git repo with `skills/<name>/SKILL.md` files plus a `registry.json` index. Any provider above works.
- **Profile.** A named bundle of `(identity + remotes)`. One profile per org; switch with `quay profile use <name>` or override per command via `--profile=<name>` / `QUAY_PROFILE`.
- **Skill.** A Markdown file with YAML frontmatter (`name`, `description`, `version`, `tags`, …). Lives at `.agents/skills/<name>/SKILL.md` after install. Tool-specific mirrors (`.claude/skills/`, `.codex/skills/`, `.cursor/skills/`) handled by `quay link`.
- **Mirror roots.** quay scans all four roots — `.agents/skills/`, `.claude/skills/`, `.codex/skills/`, `.cursor/skills/` — and deduplicates by folder name. Skills appearing in multiple roots show a drift badge when content differs.
- **Scan.** `quay scan` walks all four mirror roots and labels each skill as `Local`, `Installed`, `InstalledModified`, or `PushedLocal`. Shows `MIRRORS` and drift columns.

## Repo layout

```
apps/cli/                  Rust workspace
├── crates/quay-core/      domain logic (config, registry, fetcher, manager,
│                          scanner, providers/{github,gitlab,bitbucket,azure})
├── crates/quay-cli/       clap subcommands + ratatui TUI
└── crates/quay/           binary
.github/workflows/         release.yml — cargo-dist matrix + Homebrew publish
.agents/                   shared agent rules + skills (tool-neutral)
.claude/                   Claude Code-specific configuration
```

## Breaking Changes (v0.2.0)

- `quay sync` removed. Skills are tracked by git history — commit your `.agents/skills/` changes normally.
- `quay create` removed. Write `SKILL.md` directly (with your editor, AI agent, or any tool).
- `skills.lock.json` no longer written. If a legacy lockfile exists, quay prints a one-time notice and instructions to delete it.
- `quay add` JSON output changed: no longer returns `version`/`remote` fields (those came from the lockfile).

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
