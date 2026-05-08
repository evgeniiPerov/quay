# `.claude/` — Claude Code-Specific Configuration

Holds **only** what Claude Code needs that no other assistant uses. Universal stuff (skills, rules, commands, subagent definitions) lives in [`../.agents/`](../.agents/).

## Layout

```
.claude/
├── README.md            # this file
├── settings.json        # permissions, hooks, env (committed)
├── settings.local.json  # personal overrides (gitignored)
├── commands/            # Claude-only slash commands (rare — prefer .agents/commands/)
└── hooks/               # shell scripts wired into Claude Code's hook system
```

## What goes here vs `.agents/`

| Concern                                 | Lives in       |
|-----------------------------------------|----------------|
| Permissions / hooks / env / status line | `.claude/`     |
| Skills, rules, slash commands, agents   | `.agents/`     |
| Project-wide instructions               | `../AGENTS.md` |

If a piece of config could plausibly be reused by Codex, Cursor, or Copilot, it does **not** belong here.

## settings.json conventions

- Reference `.agents/` paths from hooks where possible so behavior stays consistent across tools.
- Keep `permissions.allow[]` minimal and project-relevant.
- Personal overrides go in `settings.local.json` (already gitignored by Claude Code).

See [`docs/superpowers/plans/2026-05-08-plan-2-agents-claude-split.md`](../docs/superpowers/plans/2026-05-08-plan-2-agents-claude-split.md) for the rationale.
