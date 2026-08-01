//! Abstraction over harbor (hub) git history. Production impl shells `git`
//! (added in Task 7); a fake is provided for unit tests.

use crate::error::{QuayError, Result};
use crate::reconcile::verdict::CommitId;
use std::path::{Path, PathBuf};
use std::process::Command;

/// One commit that touched the skill file, newest-first ordering by caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    pub id: CommitId,
    /// ISO-8601 committer date, e.g. "2026-05-10T12:00:00+00:00".
    pub committed_date: String,
}

/// Minimal git surface needed for baseline derivation.
pub trait HarborHistory {
    /// Full SHA of harbor HEAD.
    fn head_sha(&self) -> Result<CommitId>;
    /// Bytes of `skill_path` at `rev`; `None` if the path does not exist there.
    fn bytes_at(&self, rev: &str, skill_path: &str) -> Result<Option<Vec<u8>>>;
    /// Repo-relative paths of every **regular file** under the directory
    /// `prefix` at `rev`, sorted. Empty when the directory does not exist there
    /// — a skill is a directory, and `bytes_at` can only answer about a path
    /// already known.
    ///
    /// Symlinks and gitlinks are excluded, matching the file set
    /// `skill_files::collect_skill_files` collects locally. Including them would
    /// report a difference no `quay add` could resolve, since the installer
    /// skips them too.
    ///
    /// `prefix` matches whole path components: `skills/x` must not return files
    /// under `skills/x-tra`.
    fn paths_at(&self, rev: &str, prefix: &str) -> Result<Vec<String>>;
    /// Commits touching `skill_path`, newest-first. `skill_path` may be a
    /// directory, in which case any file under it counts.
    fn commits_touching(&self, skill_path: &str) -> Result<Vec<Commit>>;
    /// True iff `a` is an ancestor of `b`.
    fn is_ancestor(&self, a: &CommitId, b: &CommitId) -> Result<bool>;
}

/// Clones the harbor once into a tempdir and answers history queries by
/// shelling `git`. Partial clone (`--filter=blob:none --no-checkout`); falls
/// back to a full clone if the server rejects the filter.
pub struct GitHarborHistory {
    repo: PathBuf,
    _tmp: tempfile::TempDir,
}

/// Runs `git -C <repo> <args>` and returns the full output.
///
/// Returns `Err(QuayError::Io)` if the process cannot be spawned, and
/// `Err(QuayError::Reconcile)` if the process exits with a non-zero status.
///
/// Note: `is_ancestor` deliberately bypasses this helper. `merge-base
/// --is-ancestor` uses its exit status as the *answer*, not as a failure
/// signal. Every other query here treats a non-zero exit as an error;
/// `bytes_at` decides absence from `ls-tree` output rather than an exit code
/// precisely so it can keep doing that.
fn git(repo: &Path, args: &[&str]) -> Result<std::process::Output> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .map_err(|source| QuayError::Io {
            path: "git".into(),
            source,
        })?;
    if !out.status.success() {
        return Err(QuayError::Reconcile(format!(
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    Ok(out)
}

/// One `git ls-tree -r -z` record: `<mode> SP <type> SP <sha> TAB <path>`.
/// Returns the path only for a regular file blob (`100644`/`100755`); symlinks
/// (`120000`), gitlinks (`160000`) and anything unrecognized are dropped.
fn parse_ls_tree_regular_file(record: &str) -> Option<String> {
    let (meta, path) = record.split_once('\t')?;
    let mode = meta.split(' ').next()?;
    matches!(mode, "100644" | "100755").then(|| path.to_string())
}

impl GitHarborHistory {
    /// Clone the harbor at `url` (optionally a specific `branch`) into a
    /// temporary directory. Attempts a partial clone first; falls back to a
    /// full clone if the server rejects `--filter=blob:none`.
    pub fn clone_harbor(url: &str, branch: Option<&str>) -> Result<Self> {
        let tmp = tempfile::tempdir().map_err(|source| QuayError::Io {
            path: "tempdir".into(),
            source,
        })?;
        let dest = tmp.path().join("harbor");
        let try_clone = |filtered: bool| -> std::io::Result<std::process::ExitStatus> {
            let mut c = Command::new("git");
            c.arg("clone");
            if filtered {
                c.arg("--filter=blob:none").arg("--no-checkout");
            }
            if let Some(b) = branch {
                c.arg("--branch").arg(b);
            }
            c.arg(url).arg(&dest).status()
        };
        match try_clone(true) {
            Err(source) => {
                return Err(QuayError::Io {
                    path: "git clone".into(),
                    source,
                });
            }
            Ok(s) if s.success() => { /* partial clone ok */ }
            Ok(_) => {
                let _ = std::fs::remove_dir_all(&dest);
                let s = try_clone(false).map_err(|source| QuayError::Io {
                    path: "git clone".into(),
                    source,
                })?;
                if !s.success() {
                    return Err(QuayError::Reconcile(format!("git clone {url} failed")));
                }
            }
        }
        Ok(Self {
            repo: dest,
            _tmp: tmp,
        })
    }

    /// True when the clone really is blobless — it has a promisor remote *and*
    /// some object reachable from HEAD is absent locally, to be fetched on
    /// demand.
    ///
    /// Neither half answers this alone, and each fails in the opposite
    /// direction (all verified on git 2.55.0):
    ///
    /// - **`remote.origin.promisor`.** git writes `promisor=true` and
    ///   `partialclonefilter=blob:none` into the clone config *even when it
    ///   just told you it ignored the filter*. The config records what was
    ///   asked for, not what happened — false positive.
    /// - **`rev-list --objects --missing=print`.** It prefixes with `?` every
    ///   object git knows about and does not have, whatever the reason. Delete
    ///   blobs from a genuinely full clone and it prints `?` lines too — also a
    ///   false positive.
    ///
    /// Together they are decisive: a promisor clone that is missing objects is
    /// missing them *because* it is blobless.
    ///
    /// The reason this predicate is needed at all: a server without
    /// `uploadpack.allowFilter` does not reject `--filter=blob:none`. It warns
    /// "filtering not recognized by server, ignoring" and hands back a **full**
    /// clone, successfully. So the clone's exit status cannot distinguish the
    /// two either, and until the filtering fixture landed alongside this
    /// method, every test in this repo had silently taken the full-clone path
    /// while production ran blobless against GitHub.
    ///
    /// That says nothing about `clone_harbor`'s fallback, which has *not* been
    /// shown to be unreachable: a client older than git 2.19 rejects `--filter`
    /// before any network round-trip, a transient failure during the filtered
    /// attempt falls through to it regardless, and JGit-backed servers (Gerrit,
    /// Bitbucket Server) are untested and need not copy C git's ignore-and-warn
    /// behaviour.
    ///
    /// Walks every object reachable from HEAD, so this is O(repo). It exists to
    /// let tests prove which path they are on; it is not on any hot path.
    ///
    /// Upgrade path: same as [`crate::reconcile::report_for_test`] — if
    /// `quay-core` is ever published, move this behind
    /// `#[cfg(any(test, feature = "test-util"))]` so a test-only predicate is
    /// not part of the shipped surface.
    #[doc(hidden)]
    pub fn is_partial(&self) -> bool {
        // `git config --get` exits 1 when the key is merely absent, which is
        // the ordinary full-clone answer, so this one cannot go through `git()`.
        let cfg = Command::new("git")
            .arg("-C")
            .arg(&self.repo)
            .args(["config", "--get", "remote.origin.promisor"])
            .output()
            .expect("spawn `git config --get remote.origin.promisor` in the harbor clone");
        if String::from_utf8_lossy(&cfg.stdout).trim() != "true" {
            return false;
        }
        // Test-only, and every caller has already panicked on a git failure
        // long before reaching here; `expect` names the question so a failure
        // reports the real cause instead of quietly answering "not partial".
        let out = git(
            &self.repo,
            &["rev-list", "--objects", "--missing=print", "HEAD"],
        )
        .expect("`git rev-list --objects --missing=print HEAD` in the harbor clone");
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .any(|l| l.starts_with('?'))
    }
}

impl HarborHistory for GitHarborHistory {
    fn head_sha(&self) -> Result<CommitId> {
        let o = git(&self.repo, &["rev-parse", "HEAD"])?;
        Ok(String::from_utf8_lossy(&o.stdout).trim().to_string())
    }

    fn bytes_at(&self, rev: &str, skill_path: &str) -> Result<Option<Vec<u8>>> {
        // Step 1: decide existence from the *tree*, never from the blob.
        //
        // A partial clone always has its trees locally, so `ls-tree` answers
        // without touching the network — empty output means the path genuinely
        // does not exist at `rev`, and nothing else.
        //
        // Asking the blob instead (`cat-file -e <rev>:<path>`) cannot say that:
        // it exits 128 both when the path is absent *and* when the promisor
        // remote is unreachable, so a hub outage would render as `Ok(None)` —
        // which `baseline::derive` turns into the "absent on HEAD" sentinel and
        // the report turns into "deleted or renamed on the hub".
        //
        // Output is only tested for emptiness; `-z` keeps a path containing a
        // newline from looking like content when the entry is missing.
        let listed = git(
            &self.repo,
            &["ls-tree", "--name-only", "-z", rev, "--", skill_path],
        )?;
        if listed.stdout.is_empty() {
            return Ok(None); // path genuinely absent at rev
        }
        // Step 2: the entry exists, so anything that goes wrong now is a real
        // failure — a broken object store, or a lazy blob fetch that could not
        // reach the hub. `git()` propagates it as Err with git's own stderr.
        let out = git(&self.repo, &["show", &format!("{rev}:{skill_path}")])?;
        Ok(Some(out.stdout))
    }

    fn paths_at(&self, rev: &str, prefix: &str) -> Result<Vec<String>> {
        // `ls-tree <rev> -- <path>` is a pathspec match, and a pathspec of
        // `skills/x` matches the directory itself, not `skills/x-tra`. The
        // trailing slash is trimmed for the caller's benefit, not git's: git
        // accepts either, but `read_harbor` strips this same prefix off each
        // result to get a skill-relative path.
        let prefix = prefix.trim_end_matches('/');
        // Modes are needed to drop symlinks (120000) and gitlinks (160000), so
        // this cannot use `--name-only`. `-z` makes records NUL-separated, so a
        // path containing a newline cannot split one.
        let o = git(&self.repo, &["ls-tree", "-r", "-z", rev, "--", prefix])?;
        let mut paths: Vec<String> = String::from_utf8_lossy(&o.stdout)
            .split('\0')
            .filter(|s| !s.is_empty())
            .filter_map(parse_ls_tree_regular_file)
            .collect();
        paths.sort();
        Ok(paths)
    }

    fn commits_touching(&self, skill_path: &str) -> Result<Vec<Commit>> {
        let o = git(&self.repo, &["log", "--format=%H%x1f%cI", "--", skill_path])?;
        Ok(String::from_utf8_lossy(&o.stdout)
            .lines()
            .filter_map(|l| {
                let (id, date) = l.split_once('\u{1f}')?;
                Some(Commit {
                    id: id.to_string(),
                    committed_date: date.to_string(),
                })
            })
            .collect())
    }

    /// Returns `true` iff `a` is an ancestor of `b`. Reflexive: `is_ancestor(x, x) == true`
    /// (git `merge-base --is-ancestor` semantics — a commit is its own ancestor).
    fn is_ancestor(&self, a: &CommitId, b: &CommitId) -> Result<bool> {
        let out = Command::new("git")
            .arg("-C")
            .arg(&self.repo)
            .args(["merge-base", "--is-ancestor", a, b])
            .status()
            .map_err(|source| QuayError::Io {
                path: "git merge-base".into(),
                source,
            })?;
        Ok(out.success())
    }
}

#[cfg(test)]
pub mod fake {
    use super::*;
    use std::collections::HashMap;

    /// In-memory harbor for tests. `chain` is oldest→newest commit ids; `blobs`
    /// maps (commit_id, path) -> bytes. HEAD = last of `chain`.
    pub struct FakeHarborHistory {
        pub chain: Vec<Commit>,
        pub blobs: HashMap<(String, String), Vec<u8>>,
    }

    impl HarborHistory for FakeHarborHistory {
        fn head_sha(&self) -> Result<CommitId> {
            Ok(self.chain.last().expect("non-empty chain").id.clone())
        }
        fn bytes_at(&self, rev: &str, skill_path: &str) -> Result<Option<Vec<u8>>> {
            let id = if rev == "HEAD" {
                self.head_sha()?
            } else {
                rev.to_string()
            };
            Ok(self.blobs.get(&(id, skill_path.to_string())).cloned())
        }
        fn paths_at(&self, rev: &str, prefix: &str) -> Result<Vec<String>> {
            let id = if rev == "HEAD" {
                self.head_sha()?
            } else {
                rev.to_string()
            };
            let dir = format!("{}/", prefix.trim_end_matches('/'));
            let mut paths: Vec<String> = self
                .blobs
                .keys()
                .filter(|(c, p)| *c == id && p.starts_with(&dir))
                .map(|(_, p)| p.clone())
                .collect();
            paths.sort();
            Ok(paths)
        }
        fn commits_touching(&self, _skill_path: &str) -> Result<Vec<Commit>> {
            let mut v = self.chain.clone();
            v.reverse(); // newest-first
            Ok(v)
        }
        fn is_ancestor(&self, a: &CommitId, b: &CommitId) -> Result<bool> {
            let ia = self.chain.iter().position(|c| &c.id == a);
            let ib = self.chain.iter().position(|c| &c.id == b);
            Ok(matches!((ia, ib), (Some(x), Some(y)) if x <= y))
        }
    }

    #[test]
    fn fake_head_and_ancestor() {
        let chain = vec![
            Commit {
                id: "c1".into(),
                committed_date: "2026-01-01".into(),
            },
            Commit {
                id: "c2".into(),
                committed_date: "2026-02-01".into(),
            },
        ];
        let f = FakeHarborHistory {
            chain,
            blobs: HashMap::new(),
        };
        assert_eq!(f.head_sha().unwrap(), "c2");
        assert!(f.is_ancestor(&"c1".into(), &"c2".into()).unwrap());
        assert!(!f.is_ancestor(&"c2".into(), &"c1".into()).unwrap());
    }
}
