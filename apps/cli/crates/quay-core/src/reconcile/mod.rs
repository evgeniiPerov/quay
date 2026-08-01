//! Skill version reconciliation: at `quay add` collision time, decide whether
//! the harbor copy is identical / newer / diverged from the local copy, and
//! render a diff. See docs/superpowers/specs/2026-05-15-skill-version-reconciliation-design.md.

pub mod action;
pub mod baseline;
pub mod diff;
pub mod folder;
pub mod harbor_history;
pub mod verdict;

use crate::error::Result;
use crate::reconcile::baseline::derive;
use crate::reconcile::diff::{render, Diff};
use crate::reconcile::harbor_history::HarborHistory;
use crate::reconcile::verdict::{classify, semver_hint, SemverRel, Verdict};

#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconcileReport {
    pub verdict: Verdict,
    pub semver: SemverRel,
    pub diff: Diff,
    /// HEAD bytes for `action::apply` when the user picks Replace. Empty when
    /// the skill is absent on harbor HEAD (Replace must be disabled then).
    pub head_bytes: Vec<u8>,
    /// True when the base-commit search hit [`baseline::WALK_CAP`] without
    /// matching.
    ///
    /// Without this, exhausting the walk is indistinguishable from genuinely
    /// finding no base: both land on [`Verdict::ChangedUnknownDirection`], and a
    /// caller phrasing that as "local edits present" would tell a user who never
    /// touched the skill that they edited it. A truncated search is not a
    /// conclusion, and callers must not present it as one.
    ///
    /// The folder-level twin is
    /// [`crate::reconcile::folder::FolderReport::base_search_truncated`].
    pub base_search_truncated: bool,
}

impl ReconcileReport {
    /// True when the skill no longer exists on harbor HEAD, so there is nothing
    /// upstream to take and Replace must not be offered.
    pub fn absent_on_hub(&self) -> bool {
        matches!(self.verdict, Verdict::AbsentOnHub)
    }
}

/// Compute the full report for one colliding skill. `local_bytes` is the raw
/// bytes of the local SKILL.md; `local_sha` its sha256 hex; versions are the
/// frontmatter `version` strings (advisory only).
pub fn reconcile(
    local_bytes: &[u8],
    local_sha: &str,
    harbor: &dyn HarborHistory,
    skill_path: &str,
    hub_version: &str,
    local_version: &str,
) -> Result<ReconcileReport> {
    let bl = derive(local_sha, harbor, skill_path)?;
    let head_bytes_opt = bl.head_bytes;
    let absent_on_hub = head_bytes_opt.is_none();
    let head_content_sha = bl.head_content_sha;
    let head_bytes = head_bytes_opt.unwrap_or_default();

    // A truncated search also lands on `ChangedUnknownDirection`, because "we
    // did not look far enough" is not a direction either. The two are told apart
    // by `base_search_truncated`, so that a caller never phrases an exhausted
    // budget as "local edits present".
    let (verdict, base_search_truncated) = if absent_on_hub {
        (Verdict::AbsentOnHub, false)
    } else {
        (
            classify(local_sha, &head_content_sha, bl.base),
            bl.truncated,
        )
    };
    let diff = render(&head_bytes, local_bytes); // render(old, new): hub HEAD is old, local is new
    let semver = semver_hint(hub_version, local_version);
    Ok(ReconcileReport {
        verdict,
        semver,
        diff,
        head_bytes,
        base_search_truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reconcile::baseline::content_sha256;
    use crate::reconcile::harbor_history::fake::FakeHarborHistory;
    use crate::reconcile::harbor_history::Commit;
    use std::collections::HashMap;

    fn harbor(chain: &[(&str, &str, &str)]) -> FakeHarborHistory {
        let mut blobs = HashMap::new();
        let mut commits = Vec::new();
        for (id, date, body) in chain {
            commits.push(Commit {
                id: (*id).into(),
                committed_date: (*date).into(),
            });
            blobs.insert(
                ((*id).into(), "p/SKILL.md".into()),
                body.as_bytes().to_vec(),
            );
        }
        FakeHarborHistory {
            chain: commits,
            blobs,
        }
    }

    #[test]
    fn identical_report() {
        let h = harbor(&[("c1", "d1", "v1")]);
        let r = reconcile(
            b"v1",
            &content_sha256(b"v1"),
            &h,
            "p/SKILL.md",
            "1.0.0",
            "1.0.0",
        )
        .unwrap();
        assert_eq!(r.verdict, Verdict::Identical);
        assert!(!r.absent_on_hub());
    }

    #[test]
    fn hub_newer_report_has_diff() {
        let h = harbor(&[("c1", "d1", "old"), ("c2", "d2", "new")]);
        let r = reconcile(
            b"old",
            &content_sha256(b"old"),
            &h,
            "p/SKILL.md",
            "2.0.0",
            "1.0.0",
        )
        .unwrap();
        assert!(
            matches!(r.verdict, Verdict::HubNewer { .. }),
            "expected HubNewer, got {:?}",
            r.verdict
        );
        assert_eq!(r.semver, SemverRel::HubHigher);
    }

    #[test]
    fn absent_at_head_is_its_own_verdict() {
        let h = FakeHarborHistory {
            chain: vec![Commit {
                id: "c1".into(),
                committed_date: "d1".into(),
            }],
            blobs: HashMap::new(), // no blob for ("c1","p/SKILL.md") => absent at HEAD
        };
        let r = reconcile(b"anything", "deadbeef", &h, "p/SKILL.md", "1.0.0", "1.0.0").unwrap();
        assert_eq!(r.verdict, Verdict::AbsentOnHub);
        assert!(r.absent_on_hub());
        assert!(r.head_bytes.is_empty());
    }
}
