# Azure DevOps

Azure DevOps Repos (`dev.azure.com`) and Azure DevOps Server (on-prem).

## Auth

Two options, both common:

**SSH** — generate a key, paste at User settings → SSH public keys in Azure DevOps. Test:

```sh
ssh -T git@ssh.dev.azure.com
```

**PAT over HTTPS** — generate a Personal Access Token with `Code (Read & write)` scope. Use it as the password when prompted; cache via your credential helper.

## CLI binding: `az repos`

quay uses `az repos pr create` for PR mode. Install the Azure CLI:

```sh
brew install azure-cli      # macOS
# or: curl -sL https://aka.ms/InstallAzureCLIDeb | sudo bash
az extension add --name azure-devops
```

Sign in:

```sh
az login
az devops configure --defaults organization=https://dev.azure.com/myorg project=skills
```

Without `az`, PR mode falls back to printing a compare URL.

## URL formats

Azure DevOps URLs are long. Two equivalents quay accepts:

| Form | Use |
|---|---|
| `https://dev.azure.com/<org>/<project>/_git/<repo>` | HTTPS. Auto-detected as `azuredevops`. |
| `git@ssh.dev.azure.com:v3/<org>/<project>/<repo>` | SSH. Auto-detected as `azuredevops`. |

## Add the remote

```sh
quay remote add work https://dev.azure.com/myorg/skills/_git/skills-hub --provider azuredevops --default
quay remote test work
```

## Branch policy gotchas

Azure DevOps "Branch policies" are powerful and strict:

- **Minimum reviewers** — direct push to a policied branch fails. Use `push_mode = "pr"`.
- **Build validation** — PR cannot complete until pipeline passes. quay's job ends at PR-create; you wait for the build.
- **Required reviewers** — auto-assigned, no quay involvement.

For low-friction internal hubs, exempt the hub repo from branch policies (Repos → Branches → policies) and use `push_mode = "direct"`.

## Default branch

Reads `HEAD` from the remote after clone. New repos default to `main`; older ones may be `master`. Set the default at Repos → Branches.

## Common errors

| Symptom | Fix |
|---|---|
| `az: command not found` | Install Azure CLI + `azure-devops` extension. |
| `TF401019` (no access) | PAT scope wrong; regenerate with `Code (Read & write)`. |
| `RemoteRejection: pre-receive hook` | Branch policy. Use `--push-mode pr`. |
| `pull request creation failed` from `az` | `az devops configure` defaults not set, or extension missing. Re-run setup. |

## Server (on-prem)

Same provider name (`azuredevops`). Authenticate `az` against the on-prem URL:

```sh
az devops configure --defaults organization=https://tfs.corp.example/<collection> project=skills
```

## Next

- [`quay push`](../cli/push.md).
- [Recipe: Protected default branches](../recipes/branch-policy-fallback.md).
