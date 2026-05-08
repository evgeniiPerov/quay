# quay

A git-native CLI for sharing AI agent skills (`SKILL.md` format) between people and organizations.

Treats any git repository as a "hub" of skills. Supports multi-org profiles, project-pinned hubs, offline validation, and PR-based contribute path. CLI today; TUI on the roadmap.

```text
$ quay profile add work --email me@work --remote skills=https://github.com/acme/skills.git
$ quay search csv
csv-parse        Parse CSV with type inference.
$ quay add csv-parse
installed csv-parse@0.3.1 → .agents/skills/csv-parse/
$ quay --profile personal search csv     # query a different profile in one command
```

## Status

| Plan | Scope | State |
|---|---|---|
| 1 / 1.5 | foundation, read-only CLI | ✅ |
| 2 / 2.5 | search, outdated, update, sync | ✅ |
| 3 | create, validate, push (PR-based) | ✅ |
| 4 | profiles (multi-org) | ✅ |
| 5 | mirroring (`quay link`) | ✅ |
| 6 | TUI MVP (Dashboard/Browse/Search/Installed) | ✅ |
| 6.5 | TUI Settings + profile switcher modal | ✅ |
| 6.75 | TUI Create/Push + first-run onboarding | planned |
| 7 | GitLab/Azure/Bitbucket providers + distribution | planned |

150 tests, 0 clippy warnings, MSRV `stable`. Plans live in [`docs/superpowers/plans/`](docs/superpowers/plans/). Active design doc: [`docs/superpowers/specs/2026-05-08-quay-cli-design.md`](docs/superpowers/specs/2026-05-08-quay-cli-design.md).

## Install

From source (only option until Plan 7 ships packaged releases):

```sh
git clone https://github.com/evgeniiPerov/quay.git
cd quay/apps/cli
cargo build --release
cp target/release/quay ~/.local/bin/   # or anywhere on PATH
```

Requires the `git` CLI on `PATH`. `gh` is optional — used for auto-opening pull requests on `quay push`; without it, push prints a compare URL for manual PR creation.

## Quickstart

```sh
# 1. Set up a profile and a hub
quay profile add work --email you@example.com \
  --remote main=https://github.com/your-org/skills.git

# 2. Initialize a project
cd ~/code/your-app
quay init

# 3. Browse + install skills
quay search auth
quay add jwt-verify

# 4. Author and contribute
quay create http-retries
$EDITOR .agents/skills/http-retries/SKILL.md
quay validate http-retries
quay push http-retries --bump=patch
```

See [`apps/cli/README.md`](apps/cli/README.md) for the full command list and `--json` examples.

## Concepts

- **Hub.** A git repository with `skills/<name>/SKILL.md` files plus a `registry.json` index. Any provider works (GitHub today, GitLab/Azure/Bitbucket via Plan 7).
- **Profile.** A named bundle of `(identity + remotes)`. One profile per org; switch with `quay profile use <name>` or override per command via `--profile=<name>` / `QUAY_PROFILE` env.
- **Skill.** A Markdown file with YAML frontmatter (`name`, `description`, `version`, `tags`, etc). Lives at `.agents/skills/<name>/SKILL.md` after install. Tool-specific mirrors (`.claude/skills/`, `.codex/skills/`, …) land in Plan 5.
- **Lockfile.** `.agents/skills.lock.json` pins each installed skill to a content sha. `quay sync` reproduces an exact tree from it; commit the file.

## Repo layout

```
apps/cli/                  Rust workspace
├── crates/quay-core/      domain logic (config, registry, fetcher, manager, …)
├── crates/quay-cli/       clap subcommands
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

The codebase is implemented plan-by-plan via subagent-driven development; see `.agents/rules/` for code-style and testing conventions agents follow.

## License

[MIT](LICENSE).
