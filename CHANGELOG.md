# Changelog

Notable changes per release. This file is also the source of the GitHub release
notes — `dist` reads the section matching the version being tagged.

## 0.15.0 — 2026-07-31

### Changed

- **`update` and `add --force` now notice local files the hub deleted, and
  ask what to do with them.** Both re-materialize a skill from a fresh fetch,
  and both used to silently carry forward every local file the new version
  did not contain, forever. That preserved the notes you wrote — and also
  resurrected every file the hub had removed, so `quay diff` reported it as
  local-only for the life of the install. The default is still to keep
  everything; nothing is deleted without a flag or an explicit answer.

  quay cannot tell the two apart on its own: the lockfile records a content
  hash, not a file list, so nothing on disk says what the previous install put
  there. So it asks. Files the new version lacks are listed, and you can keep
  them, delete them, pick individually, or keep these and every remaining
  skill's without being asked again.

  Without a terminal — CI, a pipe, or `--json` — nothing is deleted. The files
  are kept and a note says so, which means no existing script changes behaviour
  on upgrade. `--keep-extra` and `--delete-extra` decide it outright and work
  unattended.

  Dotfiles, dot-directories and symlinks are never offered and never deleted.
  They are outside the set quay manages, and one of them — `.quay-mirror` — is
  what marks a copy mirror as quay-managed.

## 0.14.1 — 2026-07-27

### Fixed

- **`push --bump` no longer rewrites your frontmatter.** It re-emitted the whole
  block from the parsed manifest, so the hub copy kept only the fields quay
  models: `license: MIT` came back as `license: null`, and `compatibility`,
  `metadata` and `allowed-tools` were dropped outright. `allowed-tools` is the
  only machine-readable statement of what a skill may do, and the hub copy is
  what everyone else installs. The bump now rewrites the `version:` scalar and
  passes every other byte through, CRLF frontmatter included.

## 0.14.0 — 2026-07-27

### Added

- **`quay diff <skill>`** — read-only report of how a locally installed skill
  differs from the hub's copy. `quay outdated` says *that* something differs;
  this says *what*, file by file, with a verdict derived from hub history rather
  than from version numbers (`hub is ahead by N commits`, `your copy is ahead`,
  `no longer on the hub`, …). Supports `--remote` and `--json`.

  It compares the whole skill **directory**. The reconcile engine that backs
  `quay add`'s collision prompt only ever looked at `SKILL.md`, so a skill whose
  `references/` or `scripts/` moved on the hub reported as identical. Diffs are
  pull-oriented: `+` is what the hub would give you.

### Fixed

- **`quay outdated` no longer misses a hub edit that shipped without a version
  bump.** Frontmatter skills were compared by semver alone, so a hub maintainer
  who fixed a typo or edited `references/` without touching `version` left every
  consumer reading "everything up to date" on a stale copy. Content is now
  compared for every skill format and reported as its own row —
  `differs from hub at <version>` — alongside the existing `local -> <version>`
  upgrade rows. `--json` and the `quay_outdated` MCP tool gain a
  `content_drift` field so the two reasons can be told apart.

  Drift is direction-neutral: two hashes prove the bytes differ, not which side
  changed them. `quay update` still acts on version upgrades only, so drift rows
  point at `quay add <name> --force`.

  Line endings are normalized before comparing, so a Windows checkout
  (`core.autocrlf`) does not report every installed skill as drifted. The
  published `content_hash` is unchanged — no registry needs regenerating.

## 0.13.4 — 2026-07-23

### Security

- **`registry.json` file paths are validated before anything is fetched.** The
  registry is downloaded from the remote hub, and its `files` list went straight
  into a path join unchecked — an entry like `"../../../.ssh/authorized_keys"`
  wrote there, and an absolute path escaped the skill directory entirely.
  Absolute paths, `..` components and Windows drive/UNC prefixes are now
  rejected. **If you install from a hub you do not control, update.**

### Fixed

- **Windows: frontmatter is parsed in files with CRLF line endings.** Git's
  default `core.autocrlf` rewrites line endings on checkout, so on Windows every
  frontmatter skill silently degraded to "freestyle" — losing its name,
  description and version, and listing as `unversioned`.
- **Windows: `--force` can replace a symlinked mirror.** Unlinking used
  `remove_file`, which fails on the directory symlinks and junctions that
  mirrors are on Windows, so `quay link --force`, `quay add --force` and
  `quay agents link --force` could not replace an existing mirror.
- **Dot-prefixed directories are never treated as skills.** A staging directory
  left behind by an interrupted `quay add` could appear in `quay list` as a
  skill named `.tmpAbCdEf`, and `quay link` would mirror it into every tool
  directory.

### Internal

- CI runs `cargo fmt`, `cargo clippy` and the test suite on every pull request,
  on Linux **and Windows**. The suite had never compiled on Windows, which is
  how the two bugs above went unnoticed.
- The Rust toolchain is pinned in `rust-toolchain.toml` at the repo root, so a
  new Rust release cannot turn CI red on its own and contributors resolve the
  same compiler CI uses.

> 0.13.3 was tagged but never published — the release build failed — and its
> changes are included here.

## 0.13.2 — 2026-07-23

### Fixed

- **`quay add` no longer leaves a partial skill behind when a fetch fails.**
  Files were written directly into `.agents/skills/<name>/` as they downloaded,
  so a network failure part-way through stranded a half-installed skill that
  then blocked its own retry with `AlreadyExists`. The fetch now stages in a
  temporary directory and is renamed into place only once every file has landed.
- **A failed `--force` install no longer destroys the existing skill.** The
  previous copy is moved aside and restored if the replace fails, rather than
  deleted up front.
- **`quay update` preserves files that are not in the skill manifest** — local
  notes, files dropped upstream — matching the previous overwrite-in-place
  behaviour.

## 0.13.1 — 2026-07-23

### Fixed

- `quay link` reported mirrors it had just created as no-ops when the interactive
  adopt opt-in triggered a second reconcile pass. The actions from the first pass
  are now kept.

## 0.13.0 — 2026-07-23

### Added

- **Mirror reconcile.** `quay link` scans all known tool directories on disk
  (`.agents`/`.claude`/`.codex`/`.cursor`), not just the ones in
  `[install].mirrors`, so a mirror added outside the config is still seen.
- **`install.auto_link`** (opt-in) in `.quay/config.toml`: adopt an unmanaged
  tool directory that is byte-identical to canonical, converting it to a managed
  mirror. Asked once interactively and remembered; non-interactive runs
  (`--json`, CI) never adopt.

### Changed

- **`quay link` refuses to overwrite a mirror whose content diverged from
  canonical.** Copy-strategy mirrors were previously re-materialised
  unconditionally and hand edits were lost silently. Pass `--force` to discard
  the mirror edit, or copy it into the canonical skill first.
- **`quay link check` is read-only** — it detects drift but never creates or
  overwrites.

## 0.2.0

### Removed

- `quay sync` — skills are tracked by git history; commit your `.agents/skills/`
  changes normally.
- `quay create` — write `SKILL.md` directly, with your editor, an AI agent, or
  any tool.
- `skills.lock.json` is no longer written. If a legacy lockfile exists, quay
  prints a one-time notice and instructions to delete it.

### Changed

- `quay add` JSON output no longer returns `version`/`remote` fields — those came
  from the lockfile.
