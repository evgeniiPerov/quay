# Share a skill with a team

You wrote a skill that works. You want everyone on the repo to get it with one
command, and you want new joiners to get it without asking you which hub to point
at. The trick is to put the hub in the *project* config, not in each person's
profile, and commit it.

## Point the project at the hub

```sh
quay init                                                   # creates .quay/config.toml + .agents/skills/
quay remote add team git@github.com:acme/skills-hub.git --default
quay remote test team                                       # fail now, not on first push
```

`quay remote add` writes to `.quay/config.toml` in the project, not to
`~/.config/quay/config.toml`. That is what makes this work: commit the file and
every clone of the repo knows about `team`.

```sh
git add .quay/config.toml
git commit -m "chore: point quay at the team skills hub"
```

## Publish the skill

```sh
quay validate csv-parse --strict    # offline frontmatter check, no clone
quay push csv-parse --bump minor
```

With the default `push_mode = pr`, quay clones the hub, writes
`skills/csv-parse/`, updates `registry.json`, commits on a `quay/<name>-<version>`
branch and opens a PR through `gh` / `glab` / `az`. Without the provider CLI it
prints a compare URL instead. Merge the PR — until then nobody can install it.

## Teammates install it

```sh
git pull                 # picks up .quay/config.toml
quay add csv-parse
```

The skill lands in `.agents/skills/csv-parse/`, and in any mirror directories the
project or profile configures. Whether you commit `.agents/skills/` itself is
your call — quay works either way.

When you push a new version later, teammates run `quay outdated` to see it and
`quay update csv-parse` to take it.

## Gotchas

- **Remote names collide.** A project remote overrides a profile remote with the
  same name. If a teammate already has a personal remote called `team` pointing
  somewhere else, the project's wins inside this repo — pick a name that says
  what it is.
- **Commits need an email.** `quay push` refuses to commit without one. It comes
  from the active profile (`quay profile add … --email`) or a `[user]` section in
  `.quay/config.toml` — don't put the latter in a shared file.
- **Auth is git's, not quay's.** Everyone who installs needs read access to the
  hub over whatever URL you committed. An SSH URL locks out people who only have
  HTTPS credentials; see [SSH key setup](ssh-key-setup.md).
