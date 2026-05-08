# quay CLI

Cross-platform CLI for sharing AI agent skills across GitHub-hosted hubs.

## Build

    cd apps/cli
    cargo build --release

## Quickstart

    quay init
    quay remote add my-hub https://github.com/your-org/skills.git --default
    quay add csv-parse
    quay list
    quay info csv-parse
    quay remove csv-parse

Skills install to `.agents/skills/<name>/`. Lockfile at `.agents/skills.lock.json` (commit it).

## Commands

| Command | Purpose |
|---------|---------|
| `quay init` | Create `.quay/config.toml` and `.agents/skills/`. |
| `quay remote add/list/remove` | Manage configured hubs. |
| `quay add <skill>` | Install a skill from a configured hub. |
| `quay list` | Show installed skills. |
| `quay info <skill>` | Show skill metadata without installing. |
| `quay search <query>` | Search across all configured hubs. |
| `quay outdated` | List installed skills with newer versions available. |
| `quay update [<skill>]` | Update one skill or all installed skills. `--dry-run` previews. |
| `quay remove <skill>` | Uninstall. |
| `quay sync` | Refetch any missing/drifted files at the lockfile's recorded SHA. |
| `quay create <name>` | Scaffold a new local SKILL.md template. |
| `quay validate <skill>` | Validate frontmatter offline (no network). |
| `quay push <skill>` | Push the local skill to a hub via PR (`--bump=patch\|minor\|major\|as-written`). |
| `quay profile list/current/add/remove/use/show/rename` | Manage multi-org profiles (identity + remotes per profile). |

All commands support `--json` for machine-readable output. Use `--profile=<name>` to override the active profile for one invocation, or set `QUAY_PROFILE=<name>` in the environment.

## Test

    cd apps/cli
    cargo test

## Structure

- `crates/quay-core` — domain logic (config, registry, lockfile, fetcher, manager). Pure where possible.
- `crates/quay-cli` — clap subcommands.
- `crates/quay` — binary.

## Status

This package implements Plan 1 from `docs/superpowers/plans/`. Multi-tool mirroring, profiles, push, TUI, GitLab/Azure providers, and distribution are tracked in subsequent plans.

> v0.1 fetches over HTTPS from raw.githubusercontent.com and trusts transport-layer
> integrity. Per-file content verification against an out-of-band sha is tracked for
> a later release.
