//! Derive the base commit: the harbor commit whose skill bytes hash-match the
//! local file. Pure given a `HarborHistory`.

use crate::error::Result;
use crate::reconcile::harbor_history::HarborHistory;
use crate::reconcile::verdict::{BaseFacts, BasePosition};
use sha2::{Digest, Sha256};

/// Hard cap on commits inspected (commits touching the one file). Bounds the
/// worst case; exceeding it is treated as "no base" (correct, less precise).
pub const WALK_CAP: usize = 200;

pub fn content_sha256(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    hex::encode(h.finalize())
}

/// Result of baseline derivation.
pub struct Baseline {
    /// SHA of harbor HEAD's copy of the skill (for `classify`'s identical check).
    pub head_content_sha: String,
    /// Raw bytes of harbor HEAD's copy of the skill. `None` when the skill is
    /// absent on HEAD (delete / rename); carried out so callers avoid a second
    /// `bytes_at("HEAD", ..)` fetch.
    pub head_bytes: Option<Vec<u8>>,
    /// `Some` when a base commit was found; `None` when local was edited / not
    /// from this harbor / cap exceeded.
    pub base: Option<BaseFacts>,
}

/// `skill_path` is the repo-relative path to the skill's `SKILL.md`.
pub fn derive(local_sha: &str, harbor: &dyn HarborHistory, skill_path: &str) -> Result<Baseline> {
    let head = harbor.head_sha()?;
    let head_bytes = harbor.bytes_at("HEAD", skill_path)?;
    let head_content_sha = match &head_bytes {
        Some(b) => content_sha256(b),
        // skill absent on HEAD: signal via empty sentinel; mod.rs handles it.
        None => String::new(),
    };

    if head_content_sha == local_sha && !head_content_sha.is_empty() {
        return Ok(Baseline {
            head_content_sha,
            head_bytes,
            base: None,
        }); // Identical short-circuit
    }

    let commits = harbor.commits_touching(skill_path)?;
    for commit in commits.iter().take(WALK_CAP) {
        let Some(bytes) = harbor.bytes_at(&commit.id, skill_path)? else {
            continue;
        };
        if content_sha256(&bytes) == local_sha {
            let position = if harbor.is_ancestor(&commit.id, &head)? {
                let ahead = commits.iter().take_while(|c| c.id != commit.id).count() as u32;
                let last_commit_date = commits
                    .first()
                    .map(|c| c.committed_date.clone())
                    .unwrap_or_default();
                BasePosition::AncestorOfHead {
                    commits_ahead: ahead,
                    last_commit_date,
                }
            } else {
                BasePosition::HeadAncestorOfBase
            };
            return Ok(Baseline {
                head_content_sha,
                head_bytes,
                base: Some(BaseFacts {
                    base: commit.id.clone(),
                    position,
                }),
            });
        }
    }
    Ok(Baseline {
        head_content_sha,
        head_bytes,
        base: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reconcile::harbor_history::fake::FakeHarborHistory;
    use crate::reconcile::harbor_history::Commit;
    use std::collections::HashMap;

    fn sha(s: &str) -> String {
        content_sha256(s.as_bytes())
    }

    fn harbor(chain: &[(&str, &str, &str)]) -> FakeHarborHistory {
        // each tuple: (commit_id, date, file_contents)
        let mut blobs = HashMap::new();
        let mut commits = Vec::new();
        for (id, date, body) in chain {
            commits.push(Commit {
                id: (*id).into(),
                committed_date: (*date).into(),
            });
            blobs.insert(
                ((*id).into(), "skills/x/SKILL.md".into()),
                body.as_bytes().to_vec(),
            );
        }
        FakeHarborHistory {
            chain: commits,
            blobs,
        }
    }

    #[test]
    fn short_circuits_when_local_is_head() {
        let h = harbor(&[("c1", "d1", "v1"), ("c2", "d2", "v2")]);
        let b = derive(&sha("v2"), &h, "skills/x/SKILL.md").unwrap();
        assert_eq!(b.head_content_sha, sha("v2"));
        assert!(b.base.is_none());
    }

    #[test]
    fn finds_base_and_counts_ahead() {
        let h = harbor(&[("c1", "d1", "v1"), ("c2", "d2", "v2"), ("c3", "d3", "v3")]);
        let b = derive(&sha("v1"), &h, "skills/x/SKILL.md").unwrap();
        match b.base.unwrap().position {
            BasePosition::AncestorOfHead { commits_ahead, .. } => assert_eq!(commits_ahead, 2),
            other => panic!("expected AncestorOfHead, got {other:?}"),
        }
    }

    #[test]
    fn no_base_when_local_edited() {
        let h = harbor(&[("c1", "d1", "v1"), ("c2", "d2", "v2")]);
        let b = derive(&sha("LOCALLY EDITED"), &h, "skills/x/SKILL.md").unwrap();
        assert!(b.base.is_none());
    }

    // --- purpose-built fakes for corner-case tests ---

    /// Fake where `is_ancestor` always returns false (simulates harbor rewound).
    struct AlwaysNonAncestor {
        head_bytes: Option<Vec<u8>>,
        match_id: String,
        match_bytes: Vec<u8>,
    }

    impl HarborHistory for AlwaysNonAncestor {
        fn head_sha(&self) -> Result<String> {
            Ok("head".into())
        }
        fn bytes_at(&self, rev: &str, _skill_path: &str) -> Result<Option<Vec<u8>>> {
            if rev == "HEAD" || rev == "head" {
                Ok(self.head_bytes.clone())
            } else if rev == self.match_id {
                Ok(Some(self.match_bytes.clone()))
            } else {
                Ok(None)
            }
        }
        fn commits_touching(&self, _skill_path: &str) -> Result<Vec<Commit>> {
            Ok(vec![Commit {
                id: self.match_id.clone(),
                committed_date: "2026-01-01".into(),
            }])
        }
        fn is_ancestor(&self, _a: &String, _b: &String) -> Result<bool> {
            Ok(false)
        }
    }

    #[test]
    fn head_ancestor_of_base_when_harbor_rewound() {
        // HEAD bytes differ from local so no short-circuit; commit c1 matches
        // but is_ancestor returns false → HeadAncestorOfBase.
        let local = sha("v_local");
        let h = AlwaysNonAncestor {
            head_bytes: Some("v_head".as_bytes().to_vec()),
            match_id: "c1".into(),
            match_bytes: "v_local".as_bytes().to_vec(),
        };
        let b = derive(&local, &h, "skills/x/SKILL.md").unwrap();
        match b.base.unwrap().position {
            BasePosition::HeadAncestorOfBase => {}
            other => panic!("expected HeadAncestorOfBase, got {other:?}"),
        }
    }

    #[test]
    fn cap_exceeded_returns_no_base() {
        // 201 distinct commits (none matching local) + a 202nd oldest one matching.
        // commits_touching returns newest-first; the walk is capped at WALK_CAP=200
        // so the oldest matching commit is never reached.
        let local_sha_val = sha("matching_body");
        // Build owned strings first; &str borrows from them below.
        let mut owned: Vec<(String, String, String)> = Vec::new();
        // oldest: index 0, body matches — but it is the 202nd commit (beyond cap)
        owned.push(("c_match".into(), "d0".into(), "matching_body".into()));
        // 201 newer commits that don't match
        for i in 1..=201usize {
            owned.push((format!("c{i}"), format!("d{i}"), format!("body{i}")));
        }
        // HEAD is c201 (last), body != local; chain is oldest→newest for harbor()
        let tuples: Vec<(&str, &str, &str)> = owned
            .iter()
            .map(|(a, b, c)| (a.as_str(), b.as_str(), c.as_str()))
            .collect();
        let h = harbor(&tuples);
        // Sanity: local_sha_val must not equal HEAD sha
        assert_ne!(sha("body201"), local_sha_val);
        let b = derive(&local_sha_val, &h, "skills/x/SKILL.md").unwrap();
        assert!(b.base.is_none(), "cap exceeded: expected no base");
    }

    /// Fake where HEAD has no blob for the path (absent on HEAD) but an older
    /// commit does match. Reuses FakeHarborHistory: omit the HEAD commit's blob
    /// so bytes_at("HEAD", path) returns None.
    #[test]
    fn absent_on_head_sets_empty_sentinel_and_walks() {
        // chain: c1 (older, has matching blob), c2 (HEAD, no blob for path)
        let local = sha("v1");
        let mut blobs = HashMap::new();
        // only insert blob for c1, NOT for c2 (HEAD)
        blobs.insert(
            ("c1".to_string(), "skills/x/SKILL.md".to_string()),
            "v1".as_bytes().to_vec(),
        );
        let h = FakeHarborHistory {
            chain: vec![
                Commit {
                    id: "c1".into(),
                    committed_date: "2026-01-01".into(),
                },
                Commit {
                    id: "c2".into(),
                    committed_date: "2026-02-01".into(),
                },
            ],
            blobs,
        };
        let b = derive(&local, &h, "skills/x/SKILL.md").unwrap();
        assert_eq!(b.head_content_sha, "", "absent on HEAD → empty sentinel");
        assert!(b.base.is_some(), "walk still finds older matching commit");
    }
}
