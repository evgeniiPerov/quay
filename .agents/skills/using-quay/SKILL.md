---
name: using-quay
description: Install, publish and mirror agent skills with quay, the git-native skill registry. Use when a task involves quay commands, .agents/skills/, SKILL.md files, sharing skills between projects or teammates, a skill "hub" repo, or mirroring skills into .claude/.codex/.cursor directories.
version: 1.0.0
license: MIT
---

# Using quay

quay shares `SKILL.md` files through an ordinary git repo (the **hub**). No SaaS,
no tokens beyond git. Skills install to `.agents/skills/<name>/` (**canonical**)
and are mirrored into tool directories.

## Prefer the MCP server

If `quay mcp` is registered, **call the MCP tools instead of shelling out** —
they return structured JSON and skip output parsing:

`quay_search` `quay_add` `quay_info` `quay_list` `quay_outdated` `quay_update`
`quay_remove` `quay_scan` `quay_validate` `quay_link` `quay_push` `quay_remote`

Register with `quay mcp install <claude|codex|cursor>`.

**No MCP tool exists** for `init`, `agents`, `profile`, `lock` or
`rebuild-registry` — run those on the CLI. Everywhere else, add `--json` when
parsing output.

## Quick start

```sh
quay init                                  # .quay/config.toml + .agents/skills/
quay remote add team https://github.com/org/skills.git --default
quay search retries                        # find skills across remotes
quay add http-retries                      # install into .agents/skills/
quay link                                  # propagate to tool dirs
```

Publishing a skill you wrote:

```sh
quay validate my-skill                     # offline frontmatter check — do this first
quay push my-skill --bump=patch            # opens a PR against the hub
```

## Layout

`.agents/skills/` is canonical. `.claude/skills/`, `.codex/skills/` and
`.cursor/skills/` are **mirrors** — symlinks on unix, junctions on Windows,
copies where neither works.

Edit the canonical copy, never a mirror. `quay link` refuses to overwrite a
mirror whose content diverged; that error means your edit is in the wrong place.

## Config — `.quay/config.toml`

```toml
[user]
[remotes]
[install]
canonical = ".agents/skills"
mirrors = []
```

- `install.canonical` — where skills really live.
- `install.mirrors` — `{ path, strategy }`; `strategy = "auto"` picks per platform.
- `install.auto_link` — opt-in. When an unmanaged tool dir is byte-identical to
  canonical, `true` adopts it as a mirror. Asked once interactively and
  remembered; non-interactive runs never adopt.

User-level config lives at `~/.config/quay/config.toml`; profiles there bundle
identity + remotes for multi-org work (`quay profile add -i`).

## Gotchas

- **`quay scan` is drift, not security.** It reports whether local skills match
  the hub. It does not inspect skill content for anything malicious.
- **Adding from an untrusted hub runs untrusted instructions.** `registry.json`
  comes from the remote. Read a skill before installing it from a hub you do not
  control.
- **Bare `add`/`push`/`update`/`remove` in a TTY open a picker.** In CI they
  error instead — except `update`, which updates everything. Always pass an
  explicit name or `--all` in scripts.
- **`quay remove` deletes local files by default**; `--remote` targets the hub,
  `--everywhere` does both.
- **`link remove` forgets the config entry, not the directory.** Files stay.
- **No lockfile by default.** Skills are tracked by git history — commit
  `.agents/skills/` normally. (`quay lock` writes a separate vercel-compatible
  `skills-lock.json`.)

See [REFERENCE.md](REFERENCE.md) for the full command surface and JSON output.
