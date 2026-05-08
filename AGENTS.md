# Quay

Cross-platform CLI + TUI for sharing AI agent skills (SKILL.md) across organizations and personal hubs. Like `npm` for skills, with git-native transport.

## What It Does

- Pull individual skills from GitHub-hosted hubs into a local `.agents/skills/` folder
- Push new skills back to a hub via PR
- Browse / search / install skills via interactive TUI
- Multi-hub support (configure N remote hubs per project)
- Tool-agnostic skills: same SKILL.md works for Claude Code, Codex, Cursor, Copilot, Kimi, etc.

## Repository Layout

This is a monorepo. Minimum two packages:

```
quay/
├── AGENTS.md                  # this file — universal project instructions
├── .agents/                   # universal agent config (skills, rules, commands, personas)
│   ├── README.md
│   ├── skills/                # SKILL.md workflows
│   ├── rules/                 # modular instructions
│   ├── commands/              # slash-command definitions
│   └── agents/                # subagent personas
├── .claude/                   # Claude Code-specific only (settings, hooks)
│   ├── README.md
│   ├── settings.json
│   └── hooks/
├── docs/                      # design specs, architecture, guides
│   └── superpowers/
│       ├── specs/             # design docs (YYYY-MM-DD-<topic>-design.md)
│       └── plans/             # implementation plans (YYYY-MM-DD-plan-N-*.md)
├── apps/
│   ├── cli/                   # Rust CLI + TUI binary (Cargo workspace)
│   │   ├── Cargo.toml
│   │   └── crates/
│   │       ├── quay-core/     # domain logic (config, resolver, manager)
│   │       ├── quay-cli/      # clap commands
│   │       ├── quay-tui/      # ratatui screens
│   │       └── quay/          # binary, wires above
│   └── web/                   # Next.js + shadcn site (later phase)
│       └── (TBD after CLI MVP)
└── packages/                  # shared TS packages if web ever needs them
```

### `.agents/` vs `.claude/`

We split agent configuration into a **universal** pool and a **Claude-specific** pool. This dogfoods quay's tool-agnostic skill model.

| Concern                                    | Location              |
|--------------------------------------------|-----------------------|
| Project-wide instructions                  | `AGENTS.md` (this file) |
| Skills, rules, slash commands, subagents   | `.agents/`            |
| Claude permissions, hooks, status line     | `.claude/`            |

If a config could plausibly be reused by Codex, Cursor, Copilot, Kimi, or Gemini CLI, it lives in `.agents/`. If it only makes sense for Claude Code, it lives in `.claude/`.

Older workflows that still expect `CLAUDE.md` should symlink: `ln -s AGENTS.md CLAUDE.md` (then gitignore the symlink).

### Multi-stack rules + personas

Rules and personas are organized by stack:

| Scope        | Rules                                                                       | Personas                                                       |
|--------------|------------------------------------------------------------------------------|----------------------------------------------------------------|
| Repo-wide    | `git-policy.md`, `security.md`                                               | —                                                              |
| `apps/cli/`  | `code-style.md`, `testing.md` (path-scoped to Rust)                          | `code-reviewer`, `implementer`, `tester`                       |
| `apps/web/`  | `web-code-style.md`, `web-testing.md`, `web-accessibility.md` (path-scoped)  | `react-implementer`, `web-reviewer`, `e2e-tester`, `a11y-auditor`, `perf-auditor` |

Path-scoped rules (`paths:` frontmatter) apply only when files in their glob are touched. Repo-wide rules apply always.

### Mono → poly migration trigger

Today this is a monorepo. The plan is to split `apps/web/` into a separate repo when **either** condition is met:

1. The web app crosses ~5K LOC of non-generated TypeScript.
2. The web app needs an independent deploy cadence (first production deploy is the natural cut point).

When that happens:
- New repo: `<org>/quay-web` (or whatever name).
- Shared rules + personas extract to `<org>/agents-hub`, consumed by both repos via `quay add <org>/agents-hub@<rule>`.
- This repo (`quay`) keeps the CLI + remains the source-of-truth for Rust skills.

Until the trigger is hit, both stacks live here, with path-scoped rules + personas keeping concerns separated.

See [`docs/superpowers/plans/2026-05-08-plan-2-agents-claude-split.md`](docs/superpowers/plans/2026-05-08-plan-2-agents-claude-split.md) for the full rationale and migration plan, and the per-directory READMEs in [`.agents/README.md`](.agents/README.md) and [`.claude/README.md`](.claude/README.md).

## Status

Plans 1–6.75 are **implemented**. The CLI provides:
- `init`, `remote add/list/remove` — project setup
- `profile list/add/remove/use/current/show/rename` — multi-org identities
- `add`, `list`, `remove`, `info` — single-skill lifecycle
- `search`, `outdated`, `update`, `sync` — discovery and reproducibility
- `create`, `validate`, `push` — contribute path (PR-based, via `git` + `gh` CLIs)
- `link`, `link check/add/remove` — multi-tool mirrors
- `tui` — interactive Dashboard / Browse / Search / Installed / Settings (Profiles / Remotes / Install tabs) + Create/Push (Screen 5, hybrid TUI form + `$EDITOR`) + first-run onboarding gate + profile switcher modal

All commands honor `--profile`, `--remote`, and `--json`.

Test status: 195 tests passing (3 ignored env-var/editor tests) in `apps/cli/`, 0 clippy warnings, release build produces a CLI that ignores the `QUAY_GITHUB_BASE_URL` test seam.

Plan 7 (additional providers, distribution, live remote test-connection) remains.

**Active design doc:** [`docs/superpowers/specs/2026-05-08-quay-cli-design.md`](docs/superpowers/specs/2026-05-08-quay-cli-design.md)

## Decisions Locked

| Area | Choice |
|------|--------|
| Name | `quay` (binary, repo, crate) |
| Language | Rust |
| TUI lib | `ratatui` |
| CLI lib | `clap` (derive) |
| Distribution | GitHub Releases + Homebrew tap (via `cargo-dist`) |
| Hub model | Generic — any GitHub repo can be a hub. N hubs supported per project. |
| Transport | Git-native (CLI shells `git clone` / `pull` / `push`) |
| Auth | Whatever the user's git config provides (SSH keys, credential helper, gh CLI) |
| Skill format | `SKILL.md` with YAML frontmatter (`name`, `description`, `version`, `tags`, `author`) |
| Versioning | Per-skill semver, tracked in `.agents/skills.lock.json` |
| MVP scope | Full TUI as primary UX (~6 weeks) |
| Web | Phase 2, separate package, Next.js + shadcn |

## Working in This Repo

- Use `pnpm` for any JS/TS code (web app, future tooling)
- Use `cargo` for Rust CLI
- Code style: see per-package configs (Cargo `clippy` for Rust, Biome for TS)
- Commits: user handles git themselves — assistants must NEVER run `git commit` or `git push`

## Brainstorming / Design Workflow

When iterating on design:
1. Discussion happens in the working session
2. Decisions captured here in AGENTS.md
3. Final spec written to `docs/superpowers/specs/YYYY-MM-DD-<topic>-design.md`
4. Implementation plans written to `docs/superpowers/plans/`
5. Code lives under `apps/`

## See Also

- `docs/superpowers/specs/` — design documents
- Origin context: this project was extracted from a brainstorming session in the `cms-craft` workspace and lives independently from that point forward.
