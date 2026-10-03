# Multi-org profiles

You contribute skills to your employer's hub and to your own. The two need
different commit emails, different remotes, and — the part that bites — you
never want a work skill pushed to the personal hub by accident. A **profile**
bundles an identity with its remotes; switch profiles and everything follows.

## One profile per org

```sh
quay profile add work --email you@acme.dev \
  --remote hub=git@github.com:acme/skills-hub.git --default

quay profile add personal --email you@hey.com \
  --remote hub=git@gitlab.com:you/skills.git --provider gitlab --default \
  --activate
```

Both profiles can call their remote `hub` — remotes are scoped to the profile.
`--provider`, `--push-mode`, `--direct-branch` and `--default` apply to the
`--remote` written just before them. `quay profile add -i` asks the same
questions one at a time if you'd rather not type URLs on a command line.

## Pick the profile

Four ways, highest priority first:

| How | Scope |
|---|---|
| `--profile work` | This one command. |
| `QUAY_PROFILE=work` | This shell, or one CI job. |
| `profile = "work"` in the project's `.quay/config.toml` | Everyone working in this repo. |
| `quay profile use work` | Your machine, until you switch again. |

The project pin is the one that prevents the accident: commit it in your
employer's repos and `quay push` there resolves to `work` no matter what you last
ran `profile use` on.

```toml
# .quay/config.toml
profile = "work"
```

## Check where you are

```sh
quay profile current     # the name that resolves here
quay profile show        # its email, remotes and push modes
quay profile list        # all of them, active one marked
```

Run `current` from inside the repo — it accounts for the project pin and
`QUAY_PROFILE`, which `profile list` does not.

## Gotchas

- **A pinned profile must exist.** If `.quay/config.toml` pins `work` and your
  user config has no profile by that name, quay errors instead of falling back.
  That's deliberate; create the profile.
- **More than one profile and nothing selected is an error,** not a guess. With
  exactly one profile, it's used automatically.
- **The commit author name isn't a flag.** `--email` sets the email; the name
  comes from `name` under `[profiles.<name>.user]` in
  `~/.config/quay/config.toml`, and is a placeholder until you set it there.
- **`quay remote add` does not touch profiles.** It writes the *project* config.
  To change a profile's remotes, use `quay profile edit <name> -i` or
  `--from-toml`.
- **Two keys for one host.** If both orgs live on github.com under different SSH
  keys, see [SSH key setup](ssh-key-setup.md#two-accounts-on-one-host).
