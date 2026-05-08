# `.agents/` — Universal Agent Configuration

Tool-agnostic configuration shared across **every** AI coding assistant the project supports (Claude Code, Codex, Cursor, Copilot, Kimi, Gemini CLI, etc.).

Anything Claude-specific lives in [`../.claude/`](../.claude/). Anything universal lives here.

## Layout

```
.agents/
├── README.md          # this file
├── skills/            # SKILL.md workflows (auto-invoked by capable agents)
├── rules/             # modular instruction snippets (code-style.md, testing.md, …)
├── commands/          # slash-command definitions in markdown
└── agents/            # specialized subagent personas
```

## What goes where

| Directory   | Contents                                                                 |
|-------------|--------------------------------------------------------------------------|
| `skills/`   | One subdirectory per skill, each containing a `SKILL.md` + assets        |
| `rules/`    | One markdown file per concern (e.g. `code-style.md`, `testing.md`)       |
| `commands/` | One markdown file per slash command (filename = command name)            |
| `agents/`   | One markdown file per persona (e.g. `code-reviewer.md`, `test-runner.md`)|

The root [`AGENTS.md`](../AGENTS.md) is the project-wide instruction file — equivalent to the historical `CLAUDE.md`, but tool-agnostic. Most modern agents (Codex, Cursor, Claude Code's `--agents-md` mode, Gemini CLI) read it automatically.

## Tool integration

Agents that don't natively read `.agents/` should be wired in via tool-specific configs that reference these paths:

- **Claude Code** → `.claude/settings.json` for permissions/hooks; `.claude/agents/` symlinked to `../.agents/agents/`
- **Codex** → `AGENTS.md` already supported natively (no subagent equivalent yet)
- **Cursor** → `.cursor/rules/` symlinks or imports from `.agents/rules/`
- **Copilot** → `.github/copilot-instructions.md` references `AGENTS.md`
- **Kimi / Gemini / OpenCode** → consume `.agents/skills/` directly via `npx skills add`

## Caveat: subagent format

The files in [`agents/`](agents/) use **Claude Code's** frontmatter (`tools:`, `model:`). Today, only Claude Code reads them — every other assistant ignores them. We keep the source of truth in `.agents/agents/` (and symlink it into `.claude/agents/`) on the bet that Codex, Kimi, and others will converge on a similar persona model. Until then, treat this directory as Claude-flavored even though it lives in the universal pool.

## Why this split

Quay itself is a CLI for sharing skills across organizations. We dogfood the convention: skills authored here are portable to any consumer's `.agents/skills/` regardless of which assistant they use.

See [`docs/superpowers/plans/2026-05-08-plan-2-agents-claude-split.md`](../docs/superpowers/plans/2026-05-08-plan-2-agents-claude-split.md) for the full rationale and migration plan.
