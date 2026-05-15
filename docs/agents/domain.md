# Domain Docs

How the engineering skills should consume this repo's domain documentation when exploring the codebase.

This is a **single-context** repo (one domain, no `CONTEXT-MAP.md`).

## Before exploring, read these

- **`CONTEXT.md`** at the repo root (does not exist yet — created lazily by `/grill-with-docs` when terms get resolved).
- **`docs/adr/`** — Architecture Decision Records (does not exist yet — same lazy creation).
- Existing project knowledge already lives in **`AGENTS.md`** (decisions locked, status, layout) and **`docs/superpowers/specs/`** (design docs) + **`docs/superpowers/plans/`** (implementation plans). Read those when working in an area before inventing new context.

If `CONTEXT.md` / `docs/adr/` don't exist, **proceed silently**. Don't flag their absence; don't suggest creating them upfront. The producer skill (`/grill-with-docs`) creates them lazily when terms or decisions actually get resolved.

## File structure

Single-context repo:

```
/
├── AGENTS.md                 ← project instructions, decisions locked
├── CONTEXT.md                ← glossary (created lazily)
├── docs/
│   ├── adr/                  ← decisions (created lazily)
│   └── superpowers/
│       ├── specs/            ← design docs
│       └── plans/            ← implementation plans
└── apps/cli/                 ← Rust workspace (quay-core / quay-cli / quay-tui / quay)
```

## Use the glossary's vocabulary

When your output names a domain concept (in an issue title, a refactor proposal, a hypothesis, a test name), use the term as defined in `CONTEXT.md`. Until `CONTEXT.md` exists, follow the vocabulary already established in `AGENTS.md` (e.g. *hub*, *remote*, *profile*, *skill*, *scan*, *push mode*, *direct branch*). Don't drift to synonyms.

If the concept you need isn't in the glossary yet, that's a signal — either you're inventing language the project doesn't use (reconsider) or there's a real gap (note it for `/grill-with-docs`).

## Flag ADR conflicts

If your output contradicts an existing ADR, surface it explicitly rather than silently overriding:

> _Contradicts ADR-0007 (event-sourced orders) — but worth reopening because…_
