# GitHub

GitHub-specific setup notes. Covers github.com and GitHub Enterprise (`githubenterprise`).

## Auth

quay clones over SSH or HTTPS — whatever URL you give it. SSH is simplest:

1. Generate a key: `ssh-keygen -t ed25519 -C "you@example.com"`.
2. Add the public key at <https://github.com/settings/keys>.
3. Test: `ssh -T git@github.com` (expect a "successfully authenticated" message).

HTTPS works too if you have `git config --global credential.helper` wired up (Keychain on macOS, libsecret on Linux, manager-core on Windows).

## CLI binding: `gh`

For PR creation, quay shells out to `gh`. Install:

```sh
brew install gh     # macOS
sudo apt install gh # Debian/Ubuntu (or via official .deb)
```

Authenticate once:

```sh
gh auth login
```

Pick HTTPS or SSH (matching your clone preference). Without `gh`, `quay push` in PR mode falls back to printing a compare URL — you'll need to open the PR in the browser manually.

## URL formats

| Form | Use |
|---|---|
| `git@github.com:org/repo.git` | SSH. Auto-detected as `github`. |
| `https://github.com/org/repo.git` | HTTPS. Auto-detected as `github`. |
| `git@ghe.example.com:org/repo.git` | GitHub Enterprise. Pass `--provider githubenterprise`. |

## Add the remote

```sh
quay remote add work git@github.com:my-org/skills-hub.git --provider github --default
quay remote test work    # connectivity probe
```

## Branch policy gotchas

- If `main` is protected (required reviews, status checks), `push_mode = "direct"` will fail at `git push origin main`. Use `push_mode = "pr"` (the default) or pass `--push-mode pr`.
- The PR is opened **against the default branch**, not necessarily `main`. quay reads the remote's `HEAD` ref after clone.
- The PR branch is named `quay/<skill-name>-<short-sha>`. Pruned automatically on merge if your repo has "Automatically delete head branches" enabled.

## Enterprise

GitHub Enterprise Server uses the same `gh` CLI, but you must authenticate against the EE host:

```sh
gh auth login --hostname ghe.example.com
```

Set the provider explicitly:

```sh
quay remote add ee git@ghe.example.com:platform/skills.git --provider githubenterprise --default
```

## Common errors

| Symptom | Fix |
|---|---|
| `gh: command not found` (in PR mode) | Install `gh`, or accept compare-URL fallback. |
| `Permission denied (publickey)` | SSH agent not running / key not added. `ssh-add ~/.ssh/id_ed25519`. |
| `403 Resource not accessible` (from `gh`) | PAT scope missing. Re-run `gh auth login` and grant `repo`. |
| Stale `registry.json` after push | CDN cache; wait ~60s or [`quay rebuild-registry`](../cli/rebuild-registry.md). |

## Next

- [`quay push`](../cli/push.md) — flag reference.
- [Recipe: Direct vs PR push](../recipes/direct-vs-pr-push.md).
