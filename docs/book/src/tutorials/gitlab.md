# GitLab

GitLab-specific setup. Works for gitlab.com and self-hosted GitLab CE/EE.

## Auth

SSH is the typical path:

1. `ssh-keygen -t ed25519 -C "you@example.com"`.
2. Paste the public key at <https://gitlab.com/-/user_settings/ssh_keys>.
3. Test: `ssh -T git@gitlab.com` (expect "Welcome to GitLab, @you!").

HTTPS works with a personal access token — generate one at <https://gitlab.com/-/user_settings/personal_access_tokens> with `write_repository` scope and configure your credential helper.

## CLI binding: `glab`

For MR (merge request) creation, quay shells out to `glab`. Install:

```sh
brew install glab
```

Authenticate:

```sh
glab auth login
```

Pick the right host (gitlab.com or your self-hosted instance) and method (HTTPS-token or SSH). Without `glab`, PR mode falls back to a compare URL.

## URL formats

| Form | Use |
|---|---|
| `git@gitlab.com:group/repo.git` | SSH. Auto-detected as `gitlab`. |
| `https://gitlab.com/group/repo.git` | HTTPS. Auto-detected as `gitlab`. |
| `git@git.corp.example:platform/skills.git` | Self-hosted. Pass `--provider gitlab` to be explicit. |

## Add the remote

```sh
quay remote add work git@gitlab.com:my-group/skills-hub.git --provider gitlab --default
quay remote test work
```

## Default branch quirks

- Newer projects default to `main`; older ones may still be on `master`. quay reads `HEAD` after clone, so either works without configuration.
- `Default branch` settings on the project page determine which branch your MR targets. Change it there if needed.

## Branch policy gotchas

- "Protect default branch" is the GitLab equivalent of GitHub's branch protection. `push_mode = "direct"` will fail with `remote: GitLab: You are not allowed to push code to protected branches`.
- "Approval rules" do not block `glab mr create`; they block the merge. Your MR is opened successfully, then awaits approvals.
- Squash settings come from project config; quay does not override them.

## Self-hosted

The provider name is `gitlab` regardless of where the instance lives — the URL is what matters. Authenticate `glab` against the right host:

```sh
glab auth login --hostname git.corp.example
```

## Common errors

| Symptom | Fix |
|---|---|
| `glab: command not found` | Install `glab` or accept compare-URL fallback. |
| `error: failed to push some refs to 'gitlab.com'` | Wrong key in agent, or branch protected. Check `ssh-add -l` and project settings. |
| `401 Unauthorized` from `glab` | Token expired or scope missing. `glab auth login` again. |
| MR opens but immediately closes | Project has "Auto-close MR" tied to default-branch matches; check project settings → Merge requests. |

## Next

- [`quay remote`](../cli/remote.md).
- [Multi-provider setup](multi-provider.md).
