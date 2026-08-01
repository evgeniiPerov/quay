//! Folder-level local-vs-harbor comparison.
//!
//! [`super::reconcile`] answers the same question for a single `SKILL.md`,
//! which is what the `quay add` collision path needs. A skill is a directory
//! though — `references/`, `scripts/`, assets — and when `SKILL.md` is
//! byte-identical while a sibling moved, the single-file verdict is `Identical`,
//! which is wrong rather than merely incomplete.
//!
//! Both sides are compared with line endings normalized to LF. git's default
//! `core.autocrlf` on Windows hands back CRLF at checkout while the harbor's
//! blobs hold LF, so a raw byte comparison would mark every file in every skill
//! as modified on that platform.

use crate::error::{QuayError, Result};
use crate::reconcile::diff::{render, Diff};
use crate::reconcile::harbor_history::HarborHistory;
use crate::reconcile::verdict::{
    classify, semver_hint, BaseFacts, BasePosition, SemverRel, Verdict,
};
use crate::skill_files::{collect_skill_files, content_hash_of, normalize_crlf};
use std::collections::BTreeMap;
use std::path::Path;

/// How one file differs between the local install and harbor HEAD.
///
/// The rendered diff rides on the variant rather than sitting beside it in an
/// `Option`: every difference has one and an unchanged file has none, and that
/// is a property of the kind, not an invariant callers have to be told about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    Same,
    Modified(Diff),
    /// Present on harbor, absent locally.
    OnlyOnHub(Diff),
    /// Present locally, absent on harbor.
    OnlyLocal(Diff),
}

impl Change {
    /// The rendered diff, or `None` for [`Change::Same`] — the only variant
    /// without one.
    pub fn diff(&self) -> Option<&Diff> {
        match self {
            Change::Same => None,
            Change::Modified(d) | Change::OnlyOnHub(d) | Change::OnlyLocal(d) => Some(d),
        }
    }
}

/// One file's entry in a [`FolderReport`].
#[derive(Debug, Clone)]
pub struct FileChange {
    /// Path relative to the skill directory, POSIX separators.
    pub rel: String,
    pub change: Change,
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
    /// True when the base-commit search hit [`WALK_CAP`] without matching.
    ///
    /// Without this, exhausting the walk is indistinguishable from genuinely
    /// finding no base: both land on [`Verdict::ChangedUnknownDirection`], and a
    /// caller phrasing that as "no commit matches your copy" would tell a user
    /// who never touched the skill that they edited it. A truncated search is
    /// not a conclusion, and callers must not present it as one.
    pub base_search_truncated: bool,
    /// Content hash of the local copy, in the LF-normalized space this report
    /// compares in — equal to `head_hash` iff the two copies match. Not the
    /// digest a registry publishes; see `skill_files::content_hash_of`.
    pub local_hash: String,
    /// Content hash of harbor HEAD's copy, same space as `local_hash`.
    pub head_hash: String,
}

impl FolderReport {
    /// Only the files that actually differ.
    pub fn changed(&self) -> impl Iterator<Item = &FileChange> {
        self.files
            .iter()
            .filter(|f| !matches!(f.change, Change::Same))
    }

    /// True when nothing exists under the skill's prefix on harbor HEAD —
    /// deleted or renamed upstream.
    pub fn absent_on_hub(&self) -> bool {
        matches!(self.verdict, Verdict::AbsentOnHub)
    }
}

/// Cap on the history walk when deriving the base commit.
///
/// Deliberately tighter than `baseline::WALK_CAP` (200): that walk reads one
/// blob per commit for a single `SKILL.md`, while this one reads a whole tree —
/// an `ls-tree` plus one blob per file — for every candidate commit, and walks
/// commits touching the entire directory rather than one file. Exceeding it
/// sets [`FolderReport::base_search_truncated`] rather than silently reporting
/// "no base found".
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

    // Absence comes from the listing, not from `head` being empty. Two reasons:
    // an empty tree hashes to a perfectly ordinary value, so hashes cannot show
    // it; and `read_harbor` filters dotfiles, so a hub directory holding only a
    // `.gitkeep` yields an empty map while the skill is very much still there.
    let absent_on_hub = harbor
        .paths_at("HEAD", skill_prefix.trim_end_matches('/'))?
        .is_empty();

    let local_hash = content_hash_of(&local);
    let head_hash = content_hash_of(&head);

    let mut rels: Vec<&String> = head.keys().chain(local.keys()).collect();
    rels.sort();
    rels.dedup();
    let files = rels
        .into_iter()
        .map(|rel| {
            let change = match (head.get(rel), local.get(rel)) {
                (Some(h), Some(l)) if h == l => Change::Same,
                // `render(old, new)`. This is a *pull* report: local is what you
                // have, hub is what you would get, so `+` must be the hub's
                // content. That is the opposite of the push-oriented argument
                // order `reconcile::reconcile` uses, where `+` is what you would
                // send.
                (Some(h), Some(l)) => Change::Modified(render(l, h)),
                (Some(h), None) => Change::OnlyOnHub(render(b"", h)),
                (None, Some(l)) => Change::OnlyLocal(render(l, b"")),
                (None, None) => unreachable!("rel came from one of the two maps"),
            };
            FileChange {
                rel: rel.clone(),
                change,
            }
        })
        .collect();

    let (verdict, base_search_truncated) = if absent_on_hub {
        (Verdict::AbsentOnHub, false)
    } else {
        // A truncated search also lands on `ChangedUnknownDirection`, because
        // "we did not look far enough" is not a direction either. The two are
        // told apart by `base_search_truncated`, so that a caller never phrases
        // an exhausted budget as "no commit matches your copy".
        let search = derive_base(&local_hash, &head_hash, harbor, skill_prefix)?;
        (
            classify(&local_hash, &head_hash, search.base),
            search.truncated,
        )
    };

    Ok(FolderReport {
        verdict,
        semver: semver_hint(hub_version, local_version),
        files,
        base_search_truncated,
        local_hash,
        head_hash,
    })
}

/// Folder-hash analogue of [`crate::reconcile::baseline::derive`].
///
/// The base must be derived in the SAME hash space `classify` compares in.
/// Deriving it from `SKILL.md` history and then classifying folder hashes
/// returns a false [`Verdict::ChangedUnknownDirection`] for every skill whose
/// `SKILL.md` did not move: the lookup finds nothing, and a knowable direction
/// is reported as unknown.
fn derive_base(
    local_hash: &str,
    head_hash: &str,
    harbor: &dyn HarborHistory,
    skill_prefix: &str,
) -> Result<BaseSearch> {
    if local_hash == head_hash {
        // identical: `classify` short-circuits before using base
        return Ok(BaseSearch {
            base: None,
            truncated: false,
        });
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
        return Ok(BaseSearch {
            base: Some(BaseFacts {
                base: commit.id.clone(),
                position,
            }),
            truncated: false,
        });
    }
    Ok(BaseSearch {
        base: None,
        truncated: commits.len() > WALK_CAP,
    })
}

/// Outcome of the base-commit walk. `base: None` with `truncated: true` means
/// "did not look far enough", which is a different claim from "looked at
/// everything and found nothing".
struct BaseSearch {
    base: Option<BaseFacts>,
    truncated: bool,
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
        out.insert(rel, normalize_crlf(bytes));
    }
    Ok(out)
}

/// Skill directory at `rev` on the harbor, keyed relative to the skill dir.
///
/// Dotfiles are dropped here to match [`read_local`]: a hub may carry a
/// `.gitkeep` that no install ever receives, and counting it would report a
/// difference no `quay add` could resolve. The other half of that parity —
/// symlinks and gitlinks — is enforced by `paths_at`, which lists regular file
/// blobs only.
fn read_harbor(
    harbor: &dyn HarborHistory,
    rev: &str,
    prefix: &str,
) -> Result<BTreeMap<String, Vec<u8>>> {
    let prefix = prefix.trim_end_matches('/');
    let mut out = BTreeMap::new();
    for path in harbor.paths_at(rev, prefix)? {
        // `paths_at` promises paths under `prefix`; guessing on a mismatch would
        // key the map by a repo-relative path, which then hashes as a different
        // file and shows up as both added upstream and deleted locally.
        let rel = path
            .strip_prefix(prefix)
            .ok_or_else(|| {
                QuayError::Reconcile(format!("harbor listed '{path}', not under '{prefix}'"))
            })?
            .trim_start_matches('/')
            .to_string();
        if rel.is_empty() || rel.split('/').any(|c| c.starts_with('.')) {
            continue;
        }
        // The listing just promised this path exists at this rev, so `None` is a
        // broken invariant rather than an absence — most likely a partial
        // clone's lazy blob fetch failing. Skipping it would report the file as
        // local-only, or, if every fetch fails, report a live skill as deleted
        // upstream.
        let bytes = harbor.bytes_at(rev, &path)?.ok_or_else(|| {
            QuayError::Reconcile(format!(
                "harbor listed '{path}' at {rev} but its content could not be read \
                 (a partial-clone blob fetch may have failed — check access to the hub)"
            ))
        })?;
        out.insert(rel, normalize_crlf(bytes));
    }
    Ok(out)
}
