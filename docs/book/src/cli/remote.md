# `quay remote`

Manage configured hub remotes inside the active profile.

## Usage

```text
quay remote <SUBCOMMAND>
```

Subcommands:

| Subcommand | What |
|---|---|
| `add` | Register a new remote (`<NAME> <URL>`). |
| `list` | Show every remote with provider + push_mode + default marker. |
| `test` | Run a connectivity probe (`git archive` or shallow clone). |
| `remove` | Delete a remote from the active profile. |

## Examples

```sh
quay remote add main git@github.com:corp/skills.git --provider github --default
quay remote add personal git@gitlab.com:me/skills.git --provider gitlab --push-mode pr
quay remote list
quay remote test main
quay remote remove personal
```

## `add` flags

| Flag | Effect |
|---|---|
| `--default` | Mark as the profile's default remote. |
| `--provider <KIND>` | Force provider: `github` / `githubenterprise` / `gitlab` / `bitbucket` / `azuredevops`. Auto-detected from URL if omitted. |
| `--push-mode <pr\|direct>` | Default push mode for this remote (default: `pr`). |
| `--profile`, `--user-config`, `--project`, `--json` | Standard globals. |

## When to use this vs …

- [`quay profile add`](profile.md) `… --remote` — seeds remotes during profile creation. Use `remote add` afterwards.
- [`quay rebuild-registry`](rebuild-registry.md) `<name>` — recovery flow when a hub's `registry.json` is broken.

## `remote test`

Two-step probe:

1. `git archive --remote=<URL> HEAD -- registry.json` (cheap; no clone).
2. If `archive` fails, shallow-clone (`--depth 1`) into a temp dir and look for `registry.json`.

Exits 0 on success, non-zero on either failure with a diagnostic message identifying which step failed. Useful in CI before `quay add` to fail fast on bad URLs / missing creds.

## Caveats

- A `gitlab` URL that points at a GitLab Enterprise instance still works; the provider only affects PR creation, not git clone.
- Removing the default remote leaves the profile with no default. The next push must use `--remote`.
- `test` will not catch every authentication issue (e.g. SSH known_hosts prompts) — if it succeeds but `push` fails, suspect agent forwarding / known_hosts.

## `--help` (top level)

```text
Manage configured hub remotes

Usage: quay remote [OPTIONS] <COMMAND>

Commands:
  add     Add a new remote
  test    Test connectivity to a configured remote
  list    List configured remotes
  remove  Remove a remote
  help    Print this message or the help of the given subcommand(s)
```
