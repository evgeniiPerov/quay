# Direct vs PR push

`quay push` delivers a skill to the hub one of two ways. Which one is right
depends on who reviews skills on that hub, not on taste.

| | `pr` (default) | `direct` |
|---|---|---|
| Lands on | A `quay/<name>-<version>` branch, plus a PR | The hub's default branch (or `direct_branch`) |
| Visible to `quay add` | After someone merges | Immediately |
| Needs | `gh`, `glab` or `az` to open the PR — otherwise a compare URL is printed | Only `git` |
| Works on | GitHub, GitLab, Azure DevOps; Bitbucket via compare URL | Any git host |
| Protected branches | Fine | Rejected by the host |

Use **PR** for a shared hub where a second pair of eyes on a skill matters —
a skill is instructions your colleagues' agents will follow. Use **direct** for a
personal hub, or a team hub whose policy is "trust, then revert".

## Set it per remote

```sh
quay remote add mine git@gitlab.com:you/skills.git --push-mode direct
quay remote edit team --push-mode pr
```

Or in a profile: `quay profile add … --remote hub=<url> --push-mode direct`.
The value is stored as `push_mode` on the remote.

## Override it once

```sh
quay push csv-parse --push-mode direct
quay push csv-parse --push-mode pr      # remote says direct, this one wants review
```

## Direct to a branch other than the default

Teams that integrate on `develop` can send direct pushes there:

```sh
quay remote edit team --push-mode direct --direct-branch develop
quay push csv-parse --direct-branch staging   # just this once
```

There is no push-time way back to the default branch: `--direct-branch ""` is
the same as leaving the flag off, so the remote's `direct_branch` still wins.
Clear it on the remote instead — `quay remote edit team --direct-branch ""`.

If the branch doesn't exist on the hub yet, quay creates it from the default
branch. `--direct-branch` is ignored when the push mode resolves to `pr`.

## What you'll see

PR mode prints the branch and the PR URL — or, when no provider CLI is
available, a URL to open the PR by hand. Direct mode prints the branch and a
short commit SHA. `--json` reports either as one object with a `mode` field. Both are recorded in
`~/.config/quay/push-log.json`, which is how `quay scan` knows a skill was pushed.

## Gotchas

- **`direct_branch` + `quay add`.** Installs read whatever branch is configured
  as the remote's `direct_branch`. A skill pushed directly to `develop` is
  installable from `develop` — not from the default branch until someone merges.
- **`remote edit` edits the project config only.** A remote that lives in a
  profile is changed with `quay profile edit`.
- **`rebuild-registry`** honors the same `push_mode` (and takes `--push-mode`).
  `remove --remote` / `--everywhere` does not: it always pushes directly.
