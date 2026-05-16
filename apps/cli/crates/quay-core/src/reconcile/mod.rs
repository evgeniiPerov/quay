//! Skill version reconciliation: at `quay add` collision time, decide whether
//! the harbor copy is identical / newer / diverged from the local copy, and
//! render a diff. See docs/superpowers/specs/2026-05-15-skill-version-reconciliation-design.md.

pub mod action;
pub mod baseline;
pub mod diff;
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
    /// True when the skill no longer exists on harbor HEAD.
    pub absent_on_head: bool,
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
    let absent_on_head = head_bytes_opt.is_none();
    let head_content_sha = bl.head_content_sha;
    let head_bytes = head_bytes_opt.unwrap_or_default();

    let verdict = if absent_on_head {
        Verdict::ChangedUnknownDirection {
            local_edited: false,
        }
    } else {
        classify(local_sha, &head_content_sha, bl.base)
    };
    let diff = render(&head_bytes, local_bytes); // render(old, new): hub HEAD is old, local is new
    let semver = semver_hint(hub_version, local_version);
    Ok(ReconcileReport {
        verdict,
        semver,
        diff,
        head_bytes,
        absent_on_head,
    })
}

/// Test-only constructor for [`ReconcileReport`].
///
/// Because `ReconcileReport` is `#[non_exhaustive]`, external crates (such as
/// `quay-cli`'s test suite) cannot construct it with a struct literal. This
/// function provides the minimal cross-crate escape hatch needed for unit tests
/// without weakening the `#[non_exhaustive]` guarantee for production code.
///
/// Upgrade path: if `quay-core` is ever published to crates.io, move this
/// behind `#[cfg(any(test, feature = "test-util"))]` so the test-only
/// constructor is not part of the shipped public surface.
#[doc(hidden)]
// test-only constructor; ReconcileReport is #[non_exhaustive]
pub fn report_for_test(verdict: verdict::Verdict, absent_on_head: bool) -> ReconcileReport {
    ReconcileReport {
        verdict,
        semver: verdict::SemverRel::Unparseable,
        diff: diff::Diff::Text(String::new()),
        head_bytes: Vec::new(),
        absent_on_head,
    }
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
        assert!(!r.absent_on_head);
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
    fn absent_on_head_gives_changed_unknown_direction() {
        let h = FakeHarborHistory {
            chain: vec![Commit {
                id: "c1".into(),
                committed_date: "d1".into(),
            }],
            blobs: HashMap::new(), // no blob for ("c1","p/SKILL.md") => absent at HEAD
        };
        let r = reconcile(b"anything", "deadbeef", &h, "p/SKILL.md", "1.0.0", "1.0.0").unwrap();
        assert_eq!(
            r.verdict,
            Verdict::ChangedUnknownDirection {
                local_edited: false
            }
        );
        assert!(r.absent_on_head);
        assert!(r.head_bytes.is_empty());
    }
}
