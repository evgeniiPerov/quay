# Protected default branches

Your hub's `main` requires reviews or status checks, and someone set the remote
to `push_mode = "direct"`. The push gets as far as `git push` and the host says
no. quay doesn't silently retry another way — a direct push and a PR have
different review semantics, and switching between them is your call.

## What you'll see

The hub's rejection, wrapped with a hint:

```text
error: config validation: direct push to 'main' failed: git command failed at git push origin main: <git's error>; if the branch is protected, set this remote's push_mode = pr
```

Nothing reached the hub. The local skill is untouched.

## Fix it for this push

```sh
quay push csv-parse --push-mode pr
```

## Fix it for good

```sh
quay remote edit team --push-mode pr
```

For a remote defined in a profile rather than the project, use
`quay profile edit <name> -i`. `pr` is the default when `push_mode` is absent,
so deleting the line from the TOML works too.

## Keep direct, change the target

If your team's rule is "anything may land on `develop`, `main` is protected",
keep direct mode and aim it at the unprotected branch:

```sh
quay remote edit team --direct-branch develop
quay push csv-parse                           # → develop
```

quay creates `develop` from the default branch if it doesn't exist yet. Note
that `quay add` and `quay outdated` then read the hub from `develop` too, so
consumers see the skill as soon as it's pushed — not when `develop` merges to
`main`.

## Other commands that push

`quay rebuild-registry` writes to the hub in the remote's `push_mode` and fails
similarly against a protected branch; its hint says to pass `--push-mode pr`,
which works as a one-off.

`quay remove --remote` / `--everywhere` always pushes directly, whatever
`push_mode` says, so against a protected branch it fails with no PR fallback.
Remove the skill by PR by hand there.

## Gotchas

- **Required signed commits.** quay commits with your `git` and your git
  config, setting only the author name and email, so signing follows your
  `commit.gpgsign` setting. If the hub requires signatures, turn signing on in
  git — quay has no flag for it.
- **`quay remote test` won't warn you.** It only reads; branch protection only
  shows up on write.
