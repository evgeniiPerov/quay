# Use with AI agents (MCP)

`quay` ships an [MCP](https://modelcontextprotocol.io) server. Run `quay mcp` and it speaks the Model Context Protocol over stdio, exposing the skill-registry operations as **structured tools** an AI agent calls directly — returning JSON instead of the human table, so the agent reasons over typed results rather than scraping stdout.

It is cross-client: one server, every MCP client (Claude Code, Codex, Cursor, VS Code, opencode, Devin, …).

## Quick start

Register quay with your client, then the 12 `quay_*` tools appear in-session.

```sh
# Claude Code — one command:
quay mcp install claude          # prints:  claude mcp add -s user quay -- quay mcp

# Anything else — print the config snippet and paste it where it says:
quay mcp install codex
quay mcp install cursor
quay mcp install vscode
quay mcp install opencode
quay mcp install devin
quay mcp install generic         # universal stdio shape for any other client
```

`quay mcp install <client>` only **prints** the snippet (and where it goes) — it never edits your client's files.

## Tools

Each tool carries MCP annotations so the *client* decides what to auto-run versus confirm. The annotation mirrors blast radius — the agent can't silently publish.

| Tool | Does | Annotation |
|---|---|---|
| `quay_search` | search hubs for skills | read-only, network |
| `quay_info` | metadata for one skill | read-only, network |
| `quay_list` | skills installed here (canonical) | read-only |
| `quay_outdated` | installed skills with newer hub versions | read-only, network |
| `quay_scan` | every `SKILL.md` on disk (all mirrors) | read-only |
| `quay_validate` | check a skill's frontmatter | read-only |
| `quay_add` | install a skill | write, network |
| `quay_link` | mirror into `.claude/`, `.codex/`, … | write |
| `quay_update` | pull newer version | write, network |
| `quay_remove` | uninstall a skill | write, **destructive** |
| `quay_push` | publish a skill (opens a PR / direct push) | write, network, **outward** |
| `quay_remote` | add a hub remote to project config | write, network |

`quay_add` is the workhorse: an agent that hits a capability gap mid-task can search → add → the skill is live in the session.

## How it works

- The server links `quay-core` directly (same code the CLI uses) — no subprocess, no parsing.
- It runs in the client's working directory. Tools operate on **that** project's `.quay/config.toml`. `search`/`add`/`outdated`/`push` need remotes configured there — see [`remote`](../cli/remote.md) and [`config.toml`](../reference/config-toml.md).
- Transport is stdio. No ports, no auth — it inherits your existing `git` credentials for hub access.
- Confirmation is the client's job (driven by the annotations above); the server never prompts.

## Per-client config shapes

`quay mcp install` emits the right shape per client. They differ:

| Client | File | Wrapper key | Notes |
|---|---|---|---|
| Claude Code | — | — | `claude mcp add -s user quay -- quay mcp` (CLI, user scope) |
| Codex | `~/.codex/config.toml` | `[mcp_servers.quay]` | TOML |
| Cursor | `.cursor/mcp.json` | `mcpServers` | |
| VS Code | `.vscode/mcp.json` | `servers` | requires `"type": "stdio"`; also `code --add-mcp` |
| opencode | `opencode.json` | `mcp` | `"type": "local"`, `command` is an **array** |
| Devin | `.devin/config.json` | `mcpServers` | cloud Devin needs `quay` installed in its VM |
| generic | any | varies | universal stdio shape; check the client's docs for the exact key |

## Verify

Without a live client, drive the protocol over stdin:

```sh
printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"smoke","version":"0"}}}' | quay mcp
```

A clean `initialize` response with `"serverInfo":{"name":"quay-mcp",…}` on a single stdout line means the server is healthy.

## Caveats

- `quay mcp` is a hidden subcommand (not in top-level `--help`) — it's meant for clients, not interactive use. Running it bare blocks waiting on stdin.
- stdout is the protocol channel: the server writes only MCP frames there; diagnostics go to stderr.
- `quay_push` is annotated outward — a well-behaved client always asks before it opens a PR.
