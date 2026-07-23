# quay reference

Every command takes the globals `--project <dir>`, `--user-config <path>`,
`--profile <name>`, `--json`.

## Commands

| Command | What it does |
|---|---|
| `init` | Create `.quay/config.toml` and `.agents/skills/` |
| `remote` | Manage hub remotes (`add`, `list`, `test`, `edit`, `remove`) |
| `add <name>` | Install a skill from a configured remote |
| `list` | List installed skills |
| `remove <name>` | Remove locally; `--remote` for the hub, `--everywhere` for both |
| `info <name>` | Show metadata without installing |
| `search <query>` | Search across configured remotes |
| `outdated` | Installed skills with newer versions available |
| `update [name]` | Update to latest; bare + non-TTY updates all |
| `scan` | Discover local skills and report sync status |
| `lock` | Generate/verify `skills-lock.json` (vercel-compatible) |
| `validate <name>` | Offline frontmatter check, no network |
| `push <name>` | Push to a hub via PR, or directly if the remote says so |
| `profile` | Multi-org identities + remote bundles |
| `rebuild-registry` | Rebuild a hub's `registry.json` from disk and push it |
| `link` | Apply/verify mirrors against tool dirs on disk |
| `agents` | Mirror into coding-agent dirs from the built-in registry |
| `mcp` | Run the MCP server; `mcp install <client>` registers it |

## Flags worth knowing

- `push --bump=<major|minor|patch>` — version the skill as you push.
- `push --push-mode=direct` — commit straight to the branch instead of a PR.
  Requires the remote to allow it.
- `add --force` — overwrite an existing skill. Without it, a collision errors.
- `link --force` — discard a diverged mirror and re-materialise from canonical.
  **Destructive**: the mirror-side edit is lost.
- `link check` — read-only drift check. Exit 0 clean, non-zero drifted. The CI gate.
- `agents list` — show known agents, `●` marks ones detected on this machine.
- `agents link -a <id>` — mirror into that agent's directory.

## Interactive vs non-TTY

| Command | `name` given | `-i` | bare TTY | bare non-TTY |
|---|---|---|---|---|
| `add` | install | picker | picker | error |
| `push` | push | picker | picker | error |
| `remove` | remove | picker | picker | error |
| `update` | update | picker | picker | **updates all** |

Pipes, redirects and CI runners all count as non-TTY.

## Bulk add collisions

`quay add -i` asks once how to handle skills that already exist: update all,
skip all, or prompt per skill. Single-skill `quay add foo` errors instead —
pass `--force`.

## Writing a SKILL.md

Frontmatter follows the [Agent Skills spec](https://agentskills.io/specification):
`name` and `description` are required; `license`, `compatibility`, `metadata`
and `allowed-tools` are optional. `name` must be lowercase, hyphen-separated,
and match the directory name.

quay is deliberately lenient when scanning: a file with no frontmatter, or a
`# /slash-command` heading, is still discovered — it just has no version.
`quay validate` tells you what it parsed.

Declare `allowed-tools` if the skill ships scripts that run shell commands or
write files. It is the only machine-readable signal of what a skill will do.

## CI

```sh
quay link check     # fail the build if mirrors drifted
quay outdated       # report skills behind the hub
quay validate <name>
```

Pair `scan` with `link check`: `scan` covers skill drift, `link check` covers
mirror drift. Neither covers the other.
