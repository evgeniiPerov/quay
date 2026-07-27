//! Folder-level local-vs-harbor comparison.
//!
//! [`super::reconcile`] answers the same question for a single `SKILL.md`,
//! which is what the `quay add` collision path needs. A skill is a directory
//! though — `references/`, `scripts/`, assets — and when `SKILL.md` is
//! byte-identical while a sibling moved, the single-file verdict is `Identical`,
//! which is wrong rather than merely incomplete.

use crate::error::{QuayError, Result};
use crate::reconcile::diff::{render, Diff};
use crate::reconcile::harbor_history::HarborHistory;
use crate::reconcile::verdict::{
    classify, semver_hint, BaseFacts, BasePosition, SemverRel, Verdict,
};
use crate::skill_files::{collect_skill_files, content_hash_of};
use std::collections::BTreeMap;
use std::path::Path;

/// How one file differs between the local install and harbor HEAD.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    Same,
    Modified,
    /// Present on harbor, absent locally.
    OnlyOnHub,
    /// Present locally, absent on harbor.
    OnlyLocal,
}

/// One file's entry in a [`FolderReport`].
#[derive(Debug, Clone)]
pub struct FileChange {
    /// Path relative to the skill directory, POSIX separators.
    pub rel: String,
    pub kind: ChangeKind,
    /// `None` when `kind` is [`ChangeKind::Same`].
    pub diff: Option<Diff>,
}

/// Result of comparing a whole skill directory against harbor HEAD.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct FolderReport {
    pub verdict: Verdict,
    /// Advisory only — frontmatter `version` strings never branch logic.
    pub semver: SemverRel,
    /// Every file on either side, sorted, unchanged ones included.
    pub files: Vec<FileChange>,
    /// True when the skill directory does not exist on harbor HEAD (deleted or
    /// renamed upstream).
    pub absent_on_head: bool,
    pub local_hash: String,
    pub head_hash: String,
}

impl FolderReport {
    /// Only the files that actually differ.
    pub fn changed(&self) -> impl Iterator<Item = &FileChange> {
        self.files.iter().filter(|f| f.kind != ChangeKind::Same)
    }
}

/// Cap on the history walk when deriving the base commit, mirroring
/// `baseline`'s. A skill whose matching commit is older than this reports
/// `ChangedUnknownDirection` rather than paying for an unbounded walk.
const WALK_CAP: usize = 50;

/// Compare `local_dir` against `skill_prefix` at harbor HEAD.
///
/// `skill_prefix` is the repo-relative skill directory on the harbor (e.g.
/// `skills/csv-parse`). Versions are frontmatter `version` strings, advisory.
pub fn folder_report(
    local_dir: &Path,
    harbor: &dyn HarborHistory,
    skill_prefix: &str,
    hub_version: &str,
    local_version: &str,
) -> Result<FolderReport> {
    let local = read_local(local_dir)?;
    let head = read_harbor(harbor, "HEAD", skill_prefix)?;

    // An empty tree hashes to a perfectly ordinary value, so "absent on harbor"
    // has to come from the listing. Comparing hashes cannot tell the two apart.
    let absent_on_head = head.is_empty();

    let local_hash = content_hash_of(&local);
    let head_hash = content_hash_of(&head);

    let mut rels: Vec<&String> = head.keys().chain(local.keys()).collect();
    rels.sort();
    rels.dedup();
    let files = rels
        .into_iter()
        .map(|rel| {
            let (kind, diff) = match (head.get(rel), local.get(rel)) {
                (Some(h), Some(l)) if h == l => (ChangeKind::Same, None),
                // `render(old, new)`. This is a *pull* report: local is what you
                // have, hub is what you would get, so `+` must be the hub's
                // content. That is the opposite of the push-oriented argument
                // order `reconcile::reconcile` uses, where `+` is what you would
                // send.
                (Some(h), Some(l)) => (ChangeKind::Modified, Some(render(l, h))),
                (Some(h), None) => (ChangeKind::OnlyOnHub, Some(render(b"", h))),
                (None, Some(l)) => (ChangeKind::OnlyLocal, Some(render(l, b""))),
                (None, None) => unreachable!("rel came from one of the two maps"),
            };
            FileChange {
                rel: rel.clone(),
                kind,
                diff,
            }
        })
        .collect();

    let verdict = if absent_on_head {
        Verdict::ChangedUnknownDirection {
            local_edited: false,
        }
    } else {
        let base = derive_base(&local_hash, &head_hash, harbor, skill_prefix)?;
        classify(&local_hash, &head_hash, base)
    };

    Ok(FolderReport {
        verdict,
        semver: semver_hint(hub_version, local_version),
        files,
        absent_on_head,
        local_hash,
        head_hash,
    })
}

/// Folder-hash analogue of [`crate::reconcile::baseline::derive`].
///
/// The base must be derived in the SAME hash space `classify` compares in.
/// Deriving it from `SKILL.md` history and then classifying folder hashes
/// returns a false `ChangedUnknownDirection { local_edited: true }` for every
/// skill whose `SKILL.md` did not move: the lookup finds nothing, and the code
/// concludes the user edited something.
fn derive_base(
    local_hash: &str,
    head_hash: &str,
    harbor: &dyn HarborHistory,
    skill_prefix: &str,
) -> Result<Option<BaseFacts>> {
    if local_hash == head_hash {
        return Ok(None); // identical: `classify` short-circuits before using base
    }
    let head_sha = harbor.head_sha()?;
    let commits = harbor.commits_touching(skill_prefix)?;
    for commit in commits.iter().take(WALK_CAP) {
        if content_hash_of(&read_harbor(harbor, &commit.id, skill_prefix)?) != local_hash {
            continue;
        }
        let position = if harbor.is_ancestor(&commit.id, &head_sha)? {
            BasePosition::AncestorOfHead {
                commits_ahead: commits.iter().take_while(|c| c.id != commit.id).count() as u32,
                last_commit_date: commits
                    .first()
                    .map(|c| c.committed_date.clone())
                    .unwrap_or_default(),
            }
        } else {
            BasePosition::HeadAncestorOfBase
        };
        return Ok(Some(BaseFacts {
            base: commit.id.clone(),
            position,
        }));
    }
    Ok(None)
}

/// Skill directory on disk, keyed by path relative to it. Uses the push file
/// set, so dotfiles and symlinks are excluded.
fn read_local(dir: &Path) -> Result<BTreeMap<String, Vec<u8>>> {
    let mut out = BTreeMap::new();
    for rel in collect_skill_files(dir)? {
        let full = dir.join(&rel);
        let bytes = std::fs::read(&full).map_err(|source| QuayError::Io {
            path: full.display().to_string(),
            source,
        })?;
        out.insert(rel, bytes);
    }
    Ok(out)
}

/// Skill directory at `rev` on the harbor, keyed relative to the skill dir.
///
/// Dotfiles are dropped to match [`read_local`]: a hub may carry a `.gitkeep`
/// that no install ever receives, and counting it would report a difference no
/// `quay add` could resolve.
fn read_harbor(
    harbor: &dyn HarborHistory,
    rev: &str,
    prefix: &str,
) -> Result<BTreeMap<String, Vec<u8>>> {
    let prefix = prefix.trim_end_matches('/');
    let mut out = BTreeMap::new();
    for path in harbor.paths_at(rev, prefix)? {
        let rel = path
            .strip_prefix(prefix)
            .unwrap_or(&path)
            .trim_start_matches('/')
            .to_string();
        if rel.is_empty() || rel.split('/').any(|c| c.starts_with('.')) {
            continue;
        }
        if let Some(bytes) = harbor.bytes_at(rev, &path)? {
            out.insert(rel, bytes);
        }
    }
    Ok(out)
}
