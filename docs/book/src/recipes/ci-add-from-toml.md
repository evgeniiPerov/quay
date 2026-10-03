# CI install via `--from-toml`

A CI job has no profile, no wizard, and no terminal. `quay profile add
--from-toml` turns a TOML file you keep in the repo into a profile in one
non-interactive step, so the job can run `quay add` like a developer would.

## The profile file

Same shape as a `[profiles.<name>]` section, minus the header — the profile name
comes from the command line, not the file:

```toml
# ci/quay-profile.toml
email = "ci-bot@acme.dev"

[remotes.team]
url = "https://github.com/acme/skills-hub.git"
default = true
# provider = "github"     # optional; auto-detected from the URL
# push_mode = "pr"        # optional; only matters if CI pushes
```

`-` reads it from stdin instead, if you'd rather template it in the job.

## The job

```sh
export QUAY_CFG="$RUNNER_TEMP/quay/config.toml"

quay --user-config "$QUAY_CFG" profile add ci --from-toml ci/quay-profile.toml --activate
quay --user-config "$QUAY_CFG" remote test team

for s in csv-parse release-notes; do
  quay --user-config "$QUAY_CFG" add "$s"
done
```

`quay add` takes one skill name per call, hence the loop.

## What you'll see

`quay add` prints `installed <name>` per skill; pass `--json` for a parseable
`{"action": "installed", "skill": …}` object instead. `quay remote test` prints a
✓ line with the size of the hub's `registry.json`, or a ✗ line naming the failure
kind (auth failed, unreachable, no registry) and exits 1.

## Gotchas

- **`profile add` refuses an existing name.** On a runner that keeps its home
  directory between jobs, the second run fails. Pointing `--user-config` at a
  per-job temp path, as above, sidesteps it.
- **No TTY changes defaults, safely.** Bare `quay add` with no skill name errors
  instead of opening the picker. A skill already on disk with different content
  is not overwritten — quay prints the verdict and diff, then exits non-zero
  telling you to re-run with `--force`. With `--force`, local files the new
  version lacks are kept unless you also pass `--delete-extra`.
- **Credentials.** quay shells out to `git`, so the job needs whatever git needs
  for that URL — a deploy key for SSH, or a token-backed credential helper for
  HTTPS. Nothing in the TOML carries a secret, and nothing should.
- **`QUAY_PROFILE`** selects a profile for every invocation if you'd rather not
  pass `--activate`; `--profile` on the command line still wins over it.
