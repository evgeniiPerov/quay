# Seed an empty hub

A hub is any git repository with skills under `skills/<name>/` and a
`registry.json` at the root. You don't build either by hand: the first
`quay push` creates both.

## Create the repository

Create it on your git host **with an initial commit** — tick "Add a README" or
equivalent. quay clones the hub and works from its default branch, so that
branch has to exist. A repository with no commits at all has none.

The README is a good place to tell people what the hub is for and how to
install from it:

```text
quay remote add team <this repo's URL> --default
quay add <skill>
```

## Register it and check access

```sh
quay remote add team git@github.com:acme/skills-hub.git --default
quay remote test team
```

`remote test` cannot pass yet — it looks for `registry.json`, and nothing has
been pushed. Read the reason it gives: *no registry* is expected here; *auth
failed* is not, and is worth fixing before the first push rather than after.

## First push

```sh
quay validate csv-parse --strict
quay push csv-parse
```

quay writes `skills/csv-parse/`, generates `registry.json` with one entry, and
commits both — through a PR or straight to the default branch, per the remote's
`push_mode`. In PR mode, merge it; the hub is empty to `quay add` until you do.

```sh
quay remote test team     # now reports registry.json and its size
quay search csv
```

## Bringing a pile of existing skills

Two routes:

- **One by one with `quay push`.** Each gets a registry entry with the right
  file list and content hash. `quay push -i` opens a checkbox picker over every
  local skill, so this is less tedious than it sounds.
- **Copy them in with git, then `quay rebuild-registry team`.** Commit
  `skills/<name>/SKILL.md` directories however you like, then let quay walk the
  tree and regenerate `registry.json` from what's actually there.

## Gotchas

- **Don't hand-write `registry.json`.** A malformed one is replaced with a fresh
  one on the next push. `rebuild-registry` rewrites it from disk, discarding any
  hand edits.
- **Direct mode against a protected default branch** fails on the very first
  push too — see [Protected default branches](branch-policy-fallback.md).
- **Bitbucket** has no provider CLI; PR mode prints a URL for you to open the
  PR in the browser.
