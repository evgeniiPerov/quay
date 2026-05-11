# Multi-provider setup

Three profiles on one machine: work GitHub, work GitLab, personal Azure DevOps. Switch with one command.

## Why three profiles?

Each profile is a bundle of:

- **Commit identity** — author email used in hub commits.
- **Remote(s)** — hub URLs plus provider kind and push mode.
- **Install settings** — mirror dirs.

You want a separate profile per identity so commits to your work hub are tagged with your work email, and personal pushes use your personal email.

## 1. First profile: work GitHub (wizard)

```sh
quay profile add -i
```

Answer:

- Name: `work-github`
- Email: `you@corp.example`
- Hub URL: `git@github.com:my-org/skills-hub.git`
- Provider: `github` (auto-detected)
- Push mode: `pr`
- Activate: yes

Confirm:

```sh
quay profile current   # → work-github
quay profile show
```

## 2. Second profile: work GitLab (explicit flags)

```sh
quay profile add work-gitlab \
  --email you@corp.example \
  --remote main=git@gitlab.com:my-org/skills-hub.git \
  --provider gitlab \
  --push-mode pr \
  --default
```

`--default` marks `main` as the profile's default remote.

## 3. Third profile: personal Azure DevOps (TOML)

Useful pattern for committable configs. Drop this in `~/code/quay-profiles/personal-azure.toml`:

```toml
name = "personal-azure"
email = "me@personal.example"

[[remotes]]
name = "main"
url = "https://dev.azure.com/myorg/skills/_git/skills-hub"
provider = "azuredevops"
push_mode = "pr"
default = true
```

Add it:

```sh
quay profile add personal-azure --from-toml ~/code/quay-profiles/personal-azure.toml
```

Stdin works too — handy for scripted bootstrap:

```sh
cat ~/code/quay-profiles/personal-azure.toml | quay profile add personal-azure --from-toml -
```

## 4. Switch profiles

Interactive picker:

```sh
quay profile use -i
```

Direct:

```sh
quay profile use work-gitlab
quay profile current   # → work-gitlab
```

For a one-shot override without persisting:

```sh
quay --profile personal-azure push early-returns
# or
QUAY_PROFILE=personal-azure quay push early-returns
```

## 5. Push the same skill to all three

Author once, publish thrice:

```sh
quay profile use work-github
quay push early-returns

quay profile use work-gitlab
quay push early-returns

quay profile use personal-azure
quay push early-returns
```

Each push records to `~/.config/quay/push-log.json` with the active profile + remote URL.

## 6. Verify

```sh
quay profile list
```

Expect three entries with the active one starred. `quay search early-returns` (run under any profile) returns hits across that profile's remotes.

## Next

- Per-provider deep-dives: [GitHub](github.md), [GitLab](gitlab.md), [Azure DevOps](azure-devops.md), [Bitbucket](bitbucket.md).
- [Recipe: Multi-org profiles](../recipes/multi-org-profiles.md).
