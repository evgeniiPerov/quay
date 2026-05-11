# Bitbucket

Bitbucket Cloud (`bitbucket.org`) and Bitbucket Data Center (server).

## The Bitbucket gotcha

Bitbucket has **no first-party CLI** with the breadth of `gh` / `glab` / `az repos`. quay does not shell out to anything. Consequence: **PR mode always falls back to a compare URL.** You open the PR in the browser manually.

If you want a hands-off flow, use `push_mode = "direct"` against an unprotected hub repo.

## Auth

SSH:

1. `ssh-keygen -t ed25519 -C "you@example.com"`.
2. Paste the public key at <https://bitbucket.org/account/settings/ssh-keys/>.
3. Test: `ssh -T git@bitbucket.org`.

HTTPS uses an **App Password**, not your account password:

1. <https://bitbucket.org/account/settings/app-passwords/> → Create app password.
2. Grant `Repositories: Read, Write`.
3. Use it as the password when prompted; cache via credential helper.

## URL formats

| Form | Use |
|---|---|
| `git@bitbucket.org:workspace/repo.git` | SSH. Auto-detected as `bitbucket`. |
| `https://bitbucket.org/workspace/repo.git` | HTTPS. Auto-detected as `bitbucket`. |

## Add the remote

```sh
quay remote add work git@bitbucket.org:my-workspace/skills-hub.git --provider bitbucket --default
quay remote test work
```

## Direct push

```sh
quay remote add work git@bitbucket.org:my-workspace/skills-hub.git \
  --provider bitbucket --push-mode direct --default
quay push early-returns                  # commit lands on default branch immediately
```

## PR mode (compare-URL fallback)

```sh
quay remote add work git@bitbucket.org:my-workspace/skills-hub.git \
  --provider bitbucket --push-mode pr --default
quay push early-returns
# Output: branch pushed, open this URL to create the PR:
# https://bitbucket.org/my-workspace/skills-hub/pull-requests/new?source=...&dest=...
```

quay pushes the branch and prints the compare URL. Click → fill out the PR description → submit.

## Branch policy gotchas

- Bitbucket "Branch restrictions" block direct pushes. Use `push_mode = "pr"`.
- "Default reviewers" auto-attach on the compare-URL form; you don't need to add them manually.
- Bitbucket squash-on-merge is per-repo. quay does not override it.

## Data Center (server)

Same provider name (`bitbucket`). URLs differ (`git@stash.corp.example:scm/proj/repo.git`); detection by host substring is approximate, so pass `--provider bitbucket` explicitly.

## Common errors

| Symptom | Fix |
|---|---|
| `Permission denied (publickey)` | SSH key not added at account-settings → SSH keys. |
| `401` over HTTPS | Using account password instead of App Password. Generate one. |
| `pre-receive hook declined` on direct push | Branch restriction. Use `--push-mode pr`. |

## Next

- [Direct vs PR push](../recipes/direct-vs-pr-push.md) — when each is appropriate.
- [`quay remote test`](../cli/remote.md) — verify connectivity before pushing.
