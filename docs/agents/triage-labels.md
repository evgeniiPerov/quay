# Triage Labels

The skills speak in terms of five canonical triage roles. This file maps those roles to the actual label strings used in this repo's issue tracker.

| Label in mattpocock/skills | Label in our tracker | Meaning                                  |
| -------------------------- | -------------------- | ---------------------------------------- |
| `needs-triage`             | `needs-triage`       | Maintainer needs to evaluate this issue  |
| `needs-info`               | `needs-info`         | Waiting on reporter for more information |
| `ready-for-agent`          | `ready-for-agent`    | Fully specified, ready for an AFK agent  |
| `ready-for-human`          | `ready-for-human`    | Requires human implementation            |
| `wontfix`                  | `wontfix`            | Will not be actioned                     |

When a skill mentions a role (e.g. "apply the AFK-ready triage label"), use the corresponding label string from this table.

These five labels do not exist in the GitHub repo yet. Create them once before first triage:

```sh
gh label create needs-triage   --color FBCA04 --description "Maintainer needs to evaluate"
gh label create needs-info     --color D4C5F9 --description "Waiting on reporter"
gh label create ready-for-agent --color 0E8A16 --description "Fully specified, AFK-ready"
gh label create ready-for-human --color 1D76DB --description "Requires human implementation"
gh label create wontfix        --color E11D21 --description "Will not be actioned"
```

Edit the right-hand column to match whatever vocabulary you actually use.
