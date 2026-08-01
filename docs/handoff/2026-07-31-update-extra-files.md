# Handoff — extra files on update (PR #30)

**Date:** 2026-07-31
**Branch:** `feat/update-extra-files` @ `e41cb72`, branched from `main` @ `0554978`
**PR:** https://github.com/evgeniiPerov/quay/pull/30 — open, MERGEABLE, CI in flight
**Follow-ups:** https://github.com/evgeniiPerov/quay/issues/31

## State

Feature complete, reviewed three times over, all fixes pushed. `cargo test --workspace` = 558 passed / 3 ignored (baseline before the branch: 522). Clippy clean with `-D warnings`. `mdbook build docs/book` clean. Version bumped to 0.15.0.

**Nothing is blocked.** The remaining work is the #31 list, which is deliberately deferred, plus watching CI.

## Where the detail already lives — do not re-derive it

| What | Where |
|---|---|
| Design rationale, decisions, rejected options | `docs/superpowers/specs/2026-07-31-update-extra-files-design.md` |
| Task-by-task implementation plan | `docs/superpowers/plans/2026-07-31-update-extra-files.md` |
| Every review finding, ruling and fix, in order | `.superpowers/sdd/2026-07-31-update-extra-files/progress.md` |
| What the feature does, for users | PR #30 body; `docs/book/src/cli/update.md`; `CHANGELOG.md` 0.15.0 |
| Deferred findings, with reasoning | Issue #31 (three already struck in a comment) |

All three `docs/superpowers/` and `.superpowers/` paths are **gitignored** — they exist only on this machine. `docs/handoff/` is tracked, so this file will show up in `git status`.

## The one thing that is genuinely unverified

The `#[cfg(windows)]` symlink-degrade path has never been compiled locally — a cross-build dies in `aws-lc-sys` for want of a mingw C compiler, so it was only type-checked via `rustc --emit=metadata`. **CI is its first real compilation.** If `test-windows` is red, that branch is the first place to look.

It is also unexecutable even when it compiles: GitHub's `windows-latest` runners are elevated, so `symlink_file` succeeds and the fallback never fires. Green Windows CI is not evidence the degrade works.

## Two judgement calls a fresh agent might otherwise re-litigate

**`--keep-extra` / `--delete-extra` carry no clap `requires = "force"` on `add`.** Deliberate. Without `--force`, `add` errors on the existing directory before the flag is reachable, so the flag is inert rather than wrong, and a `requires` would turn a harmless no-op into an argument error. A reviewer flagged this; it was overruled with that reasoning.

**`eprintln!` in `quay-core` and `quay-cli` for user-facing notes.** `.agents/rules/code-style.md` §6/§11 nominally prefers `tracing`. There is no tracing subscriber anywhere in the workspace and ~47 pre-existing `eprintln!` calls. Two separate implementers flagged the conflict rather than silently overriding it — it remains unresolved, repo-wide, and is not this branch's to fix.

## Next actions, in order

1. **Watch CI on #30** (`gh pr checks 30`). Particularly `test-windows`, per above.
2. **Merge #30** once green. `main` is unprotected, so direct merge is fine.
3. **Tag `v0.15.0`** on the merge commit to trigger the cargo-dist release. Gotchas that have bitten before are in the project memory: a failed run's rerun reuses the original token scope so a *fresh tag* is required rather than `gh run rerun`; `gh run watch --exit-status` can return 0 on a failed run, so always verify with `gh release view v0.15.0 --json assets` (expect 19 assets / 6 targets).
4. **Work #31** if wanted. The highest-value items there are the exit-0-on-abort bug and the missing `QuayError::Interrupted` variant — they travel together, since the variant is what lets the loops match on the error instead of polling a `Cell<bool>`.

## Unrelated bug found in passing, already in #31

`args.rs:36` — `#[command(visible_alias = "ls")]` sits on the `Add` variant, not `List`. **`quay ls` runs `quay add`.** Predates this branch by a long way.

## Suggested skills

- **`superpowers:subagent-driven-development`** — if picking up #31 as a batch. The ledger at `.superpowers/sdd/2026-07-31-update-extra-files/progress.md` shows exactly how this branch was run: fresh implementer per task, task-scoped review after each, one whole-branch review at the end.
- **`superpowers:brainstorming`** → **`superpowers:writing-plans`** — for any #31 item that is a behaviour question rather than a bug. The exit-code and `QuayError::Interrupted` work qualifies; it changes a user-visible contract.
- **`pr-review-toolkit:review-pr`** — the five-agent review that found the two Criticals here. Worth running again on any follow-up PR that touches `copy_missing_rec`, since that function is the deletion mechanism and every failure to copy is a deletion.
- **`superpowers:finishing-a-development-branch`** — for the merge itself.
- Do **not** reach for `superpowers:using-git-worktrees` by reflex. The user chose a plain in-place feature branch when asked, and that matches how this repo already works.
