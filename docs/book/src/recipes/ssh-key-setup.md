# SSH key setup

quay has no credentials of its own. Every clone, fetch and push is your `git`
binary, with your environment, talking to the URL in the remote's config. If
`git clone <url>` works in your shell, quay can read the hub; pushing (`quay push`)
also needs write access to the repository, which a clone does not prove. This page is the short path
to making that true over SSH.

## One key, one host

```sh
ssh-keygen -t ed25519 -C "you@acme.dev"
ssh-add ~/.ssh/id_ed25519
```

Add `~/.ssh/id_ed25519.pub` to your account on the host (GitHub: *Settings → SSH
and GPG keys*; GitLab: *Preferences → SSH Keys*; Azure DevOps: *User settings →
SSH public keys*). Then check git's side, then quay's:

```sh
ssh -T git@github.com
quay remote test team
```

`remote test` exits 0 with a ✓ line when it can read the hub's `registry.json`,
and 1 with a ✗ line saying *auth failed*, *unreachable* or *no registry*
otherwise.

Use the SSH form of the URL — `git@github.com:acme/skills-hub.git`, not
`https://…`. The provider is detected from the hostname in either form.

## Two accounts on one host

Work and personal accounts on github.com need two keys, and SSH picks a key by
host. Give each one an alias:

```text
# ~/.ssh/config
Host github-work
  HostName github.com
  User git
  IdentityFile ~/.ssh/id_ed25519_work
  IdentitiesOnly yes
```

Then use the alias in the remote URL, and **name the provider** — auto-detection
matches on hostnames like `github.com` or `gitlab.`, and `github-work` isn't
one of them:

```sh
quay profile add work --email you@acme.dev \
  --remote hub=git@github-work:acme/skills-hub.git --provider github --default
```

An unrecognized host is treated as GitHub, so a GitLab alias without
`--provider gitlab` will try to open its merge requests with `gh`.

## Gotchas

- **PRs don't use SSH.** In PR mode, quay pushes the branch over SSH, then calls
  `gh`, `glab` or `az` to open the PR — those authenticate separately
  (`gh auth login`, `glab auth login`, `az login`). With `gh`, a missing login
  falls back to a printed compare URL. With `glab` or `az`, the command fails
  after the branch is already pushed — open the MR/PR by hand, or log in and
  re-run.
- **First-contact host keys.** quay captures git's output, so an unknown-host
  prompt from SSH is easy to miss and can stall a command. Run
  `ssh -T git@<host>` once by hand to record the key in `known_hosts`.
- **No agent in the session.** Keys loaded in your desktop session aren't
  necessarily visible to a CI job, a container or an IDE terminal. If
  `ssh-add -l` lists nothing there and no `IdentityFile` (or default key file
  under `~/.ssh/`) is readable, neither git nor quay has a key to offer.
- **SSH blocked entirely?** Use the HTTPS URL with a credential helper. For
  GitHub, `gh auth setup-git` wires `gh`'s token into git, and quay inherits it.
