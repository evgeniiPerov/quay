//! Pure verdict classification. No I/O.

/// Full 40-char git commit SHA.
pub type CommitId = String;

/// Position of the derived base commit relative to harbor HEAD.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BasePosition {
    /// base is an ancestor of HEAD (harbor moved forward since install).
    AncestorOfHead {
        commits_ahead: u32,
        last_commit_date: String,
    },
    /// HEAD is an ancestor of base (harbor rewound, or local came from newer rev).
    HeadAncestorOfBase,
}

/// Facts produced by `baseline::derive` when a base commit was found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaseFacts {
    pub base: CommitId,
    pub position: BasePosition,
}

/// Advisory semver relation — display only, never branches logic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemverRel {
    HubHigher,
    LocalHigher,
    Equal,
    Unparseable,
}

/// Which way a local copy and harbor HEAD stand to each other.
///
/// Deliberately **not** `#[non_exhaustive]`, unlike the two report structs that
/// carry it. `quay-cli` is a separate crate, so marking it would force every
/// match there to grow a wildcard arm — including
/// `commands::diff::print_human`'s advice match, whose whole point is that a new
/// variant must be a compile error rather than silently getting no advice. The
/// exhaustiveness is worth more than the freedom to add a variant without a
/// breaking change; the enum is not part of a published API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Identical,
    HubNewer {
        commits_ahead: u32,
        last_commit_date: String,
        base: CommitId,
    },
    LocalAheadOrDiverged {
        base: CommitId,
    },
    /// The two copies differ and no base commit explains which way.
    ///
    /// Whether that conclusion was reached by searching the whole history or by
    /// running out of budget is not part of the verdict — see
    /// [`crate::reconcile::folder::FolderReport::base_search_truncated`].
    ChangedUnknownDirection,
    /// Nothing exists for the skill on harbor HEAD — deleted or renamed there.
    ///
    /// Never produced by [`classify`], which only ever sees hashes. Each
    /// orchestration module decides it by its own mechanism, and they differ:
    /// [`crate::reconcile::folder::folder_report`] compares a whole directory,
    /// so it asks `paths_at` for the listing under the skill prefix, while
    /// [`crate::reconcile::reconcile`] compares one file, so it takes
    /// `baseline::derive`'s `head_bytes` being `None` — a single-blob lookup.
    AbsentOnHub,
}

/// Pure classification. When `base` is `None`, the local file did not
/// content-match any harbor commit, so this function returns
/// [`Verdict::ChangedUnknownDirection`]. [`Verdict::AbsentOnHub`] is produced by
/// a later orchestration module, never by this function.
pub fn classify(local_sha: &str, head_sha: &str, base: Option<BaseFacts>) -> Verdict {
    if local_sha == head_sha {
        return Verdict::Identical;
    }
    match base {
        Some(BaseFacts {
            base,
            position:
                BasePosition::AncestorOfHead {
                    commits_ahead,
                    last_commit_date,
                },
        }) => Verdict::HubNewer {
            commits_ahead,
            last_commit_date,
            base,
        },
        Some(BaseFacts {
            base,
            position: BasePosition::HeadAncestorOfBase,
        }) => Verdict::LocalAheadOrDiverged { base },
        None => Verdict::ChangedUnknownDirection,
    }
}

/// Advisory semver comparison for display. Never affects `classify`.
pub fn semver_hint(hub: &str, local: &str) -> SemverRel {
    use semver::Version;
    use std::cmp::Ordering;
    match (Version::parse(hub), Version::parse(local)) {
        (Ok(h), Ok(l)) => match h.cmp(&l) {
            Ordering::Greater => SemverRel::HubHigher,
            Ordering::Less => SemverRel::LocalHigher,
            Ordering::Equal => SemverRel::Equal,
        },
        _ => SemverRel::Unparseable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn anc(n: u32) -> Option<BaseFacts> {
        Some(BaseFacts {
            base: "b".into(),
            position: BasePosition::AncestorOfHead {
                commits_ahead: n,
                last_commit_date: "2026-05-10".into(),
            },
        })
    }

    #[test]
    fn identical_when_local_equals_head() {
        assert_eq!(classify("aaa", "aaa", None), Verdict::Identical);
    }

    #[test]
    fn hub_newer_when_base_ancestor_of_head() {
        assert_eq!(
            classify("aaa", "bbb", anc(3)),
            Verdict::HubNewer {
                commits_ahead: 3,
                last_commit_date: "2026-05-10".into(),
                base: "b".into()
            }
        );
    }

    #[test]
    fn diverged_when_head_ancestor_of_base() {
        let base = Some(BaseFacts {
            base: "b".into(),
            position: BasePosition::HeadAncestorOfBase,
        });
        assert_eq!(
            classify("aaa", "bbb", base),
            Verdict::LocalAheadOrDiverged { base: "b".into() }
        );
    }

    #[test]
    fn unknown_when_no_base() {
        assert_eq!(
            classify("aaa", "bbb", None),
            Verdict::ChangedUnknownDirection
        );
    }

    #[test]
    fn identical_takes_precedence_over_base() {
        assert_eq!(
            classify(
                "sha",
                "sha",
                Some(BaseFacts {
                    base: "b".into(),
                    position: BasePosition::HeadAncestorOfBase,
                })
            ),
            Verdict::Identical
        );
    }

    #[test]
    fn semver_hint_variants() {
        assert_eq!(semver_hint("2.0.0", "1.0.0"), SemverRel::HubHigher);
        assert_eq!(semver_hint("1.0.0", "2.0.0"), SemverRel::LocalHigher);
        assert_eq!(semver_hint("1.0.0", "1.0.0"), SemverRel::Equal);
        assert_eq!(semver_hint("x", "1.0.0"), SemverRel::Unparseable);
    }

    #[test]
    fn semver_hint_both_unparseable() {
        assert_eq!(semver_hint("x", "y"), SemverRel::Unparseable);
    }
}
