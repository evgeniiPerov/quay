//! Abstraction over harbor (hub) git history. The production impl shells `git`;
//! a fake is provided for unit tests.
//!
//! Two invariants organize the whole file. Reads are batched per rev rather than
//! per file, because the harbor is normally a blobless clone where each file
//! read is a network round-trip. And absence is decided from the tree, never
//! from a failed read: an unreachable hub must not render as a deleted skill.

use crate::error::{QuayError, Result};
use crate::reconcile::verdict::CommitId;
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

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
    /// Every **regular file** under the directory `prefix` at `rev`, keyed by
    /// path relative to `prefix` and holding that file's bytes. Empty when the
    /// directory does not exist there.
    ///
    /// Listing and reading are one operation because they are one question — a
    /// skill is a directory, and `bytes_at` can only answer about a path already
    /// known, so every caller that listed went straight on to read. Splitting
    /// them made the caller pay a git invocation per file, and put the
    /// prefix-trimming rule in the caller as well as in each implementation. Now
    /// no caller trims anything: callers never see a repo-relative path they
    /// could key a map by.
    ///
    /// Symlinks and gitlinks are excluded, matching the file set
    /// `skill_files::collect_skill_files` collects locally. Including them would
    /// report a difference no `quay add` could resolve, since the installer
    /// skips them too. Dotfiles are *not* filtered here: whether the directory
    /// exists on the hub at all is a different question from which of its files
    /// are installable, and only the caller knows which one it is asking.
    ///
    /// `prefix` matches whole path components: `skills/x` must not return files
    /// under `skills/x-tra`.
    ///
    /// A file that is listed but whose content cannot be produced is an `Err`,
    /// never a silently absent key. Absence is a fact about the tree; a failed
    /// read is a fact about the connection, and conflating them reports an
    /// unreachable hub as a deleted skill.
    fn tree_at(&self, rev: &str, prefix: &str) -> Result<BTreeMap<String, Vec<u8>>>;
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
/// Three queries deliberately bypass this helper, and each for its own reason:
/// `is_ancestor`, because `merge-base --is-ancestor` uses its exit status as the
/// *answer*, not as a failure signal; `is_partial`'s config probe, because
/// `git config --get` exits 1 when a key is merely absent; and
/// [`GitHarborHistory::read_blobs`], because it needs stdin piped — it does
/// check the status, in a message that names the skill and rev.
///
/// Everything routed through here treats a non-zero exit as an error. `bytes_at`
/// decides absence from `ls-tree` output rather than an exit code precisely so
/// it can keep doing that.
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
/// Returns `(sha, path)` only for a regular file blob (`100644`/`100755`);
/// symlinks (`120000`), gitlinks (`160000`) and anything unrecognized are
/// dropped.
///
/// The sha is what makes one `cat-file --batch` possible: the listing already
/// names every object, so nothing else has to resolve `<rev>:<path>` again.
fn parse_ls_tree_regular_file(record: &str) -> Option<(String, String)> {
    let (meta, path) = record.split_once('\t')?;
    let mut fields = meta.split(' ');
    let mode = fields.next()?;
    let sha = fields.nth(1)?; // skip the type
    matches!(mode, "100644" | "100755").then(|| (sha.to_string(), path.to_string()))
}

/// Decodes `git cat-file --batch` output for `wanted`, in the order requested.
///
/// One record per input line: `<sha> SP <type> SP <size> LF`, then `<size>`
/// bytes, then `LF`; or `<sha> SP missing LF` when git declines to produce the
/// object. Sizes rather than delimiters are what make this safe for a blob whose
/// own bytes contain NULs, newlines, or a line shaped like a record header.
/// Duplicate shas — two files with identical content — get one record each, so
/// the stream stays positional (all verified against git 2.55.0).
///
/// Split out from the spawn so that every failure branch is unit-testable
/// without a repository. That is the point of them: they describe a git whose
/// output this code does not understand, which is precisely what cannot be
/// provoked from a working git.
///
/// `missing` is an `Err`, never an absence. Every sha came from a tree listing
/// at the same rev, so the object exists by construction. An unreachable hub
/// does not reach here at all — a failed lazy fetch kills `cat-file --batch`
/// with exit 128 and an empty stdout — so `missing` means git declined to fetch
/// rather than failed to: a corrupt object store, or `GIT_NO_LAZY_FETCH` set in
/// the environment. Either way it is a failure to read, and reporting it as "the
/// file isn't there" is what turns an operational problem into a deleted skill.
fn decode_batch(stdout: &[u8], wanted: &[(&str, &str)], rev: &str) -> Result<Vec<Vec<u8>>> {
    // Two shapes of failure, and they call for opposite reactions: the hub may
    // be unreachable, or git's batch protocol may not be what this code parses.
    // Telling a user to check hub access when the real answer is a git version
    // mismatch sends them somewhere there is nothing to find.
    let unreadable = |path: &str, why: &str| {
        QuayError::Reconcile(format!(
            "harbor listed '{path}' at {rev} but its content could not be read ({why} — \
             check access to the hub, and that the local clone is not corrupt)"
        ))
    };
    let protocol = |why: String| {
        QuayError::Reconcile(format!(
            "could not parse `git cat-file --batch` output at {rev}: {why}"
        ))
    };

    let mut rest = stdout;
    let mut blobs = Vec::with_capacity(wanted.len());
    for (sha, path) in wanted {
        let nl = rest
            .iter()
            .position(|b| *b == b'\n')
            .ok_or_else(|| unreadable(path, "git returned no record for it"))?;
        let header = String::from_utf8_lossy(&rest[..nl]).into_owned();
        rest = &rest[nl + 1..];
        // The sha a record answers with is the only proof its bytes belong to
        // this path. Everything downstream is positional — `tree_at` zips these
        // blobs against the listing — so a single desynchronized record files
        // every later file's content under its neighbour's name, and
        // `folder_report` renders a confident diff between two files that are
        // in fact identical. Cheaper to compare 40 bytes than to trust it.
        if !header.starts_with(sha) {
            return Err(protocol(format!(
                "asked for {sha} ('{path}') and git answered '{header}'"
            )));
        }
        let size = match header.split(' ').collect::<Vec<_>>()[..] {
            [_, "missing"] => return Err(unreadable(path, "git reports the object as missing")),
            [_, _, size] => size
                .parse::<usize>()
                .map_err(|_| protocol(format!("unparsable size in '{header}'")))?,
            _ => return Err(protocol(format!("unexpected record '{header}'"))),
        };
        if rest.len() < size + 1 {
            return Err(unreadable(path, "git's output ended mid-object"));
        }
        blobs.push(rest[..size].to_vec());
        rest = &rest[size + 1..]; // the record's trailing LF
    }
    // Nothing should be left. Under a correct decode this is always true, which
    // is exactly what makes it a free checksum on the whole positional walk: any
    // off-by-one that survived the per-record sha check leaves bytes behind.
    if !rest.is_empty() {
        return Err(protocol(format!(
            "{} bytes left over after the {} objects requested",
            rest.len(),
            wanted.len()
        )));
    }
    Ok(blobs)
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

    /// Runs `git -C <repo> <args>` with `input` on stdin and captures the
    /// result. Unlike [`git`], a non-zero exit is left for the caller to read:
    /// both callers need git's stderr in a message of their own.
    ///
    /// stdin is written from another thread. git drains it only while it is
    /// running, and it stops running once its stdout pipe fills, so a
    /// same-thread write-then-read deadlocks as soon as the input outgrows the
    /// pipe buffer — around 1600 shas, which is a large but perfectly ordinary
    /// skill. The write's own result is discarded: a broken pipe means git died
    /// early, and the caller reports that from the exit status and git's own
    /// stderr rather than as an `EPIPE` with no explanation.
    fn git_with_stdin(&self, args: &[&str], input: String) -> Result<std::process::Output> {
        let io_err = |source| QuayError::Io {
            path: format!("git {}", args.join(" ")),
            source,
        };
        let mut child = Command::new("git")
            .arg("-C")
            .arg(&self.repo)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(io_err)?;
        let mut stdin = child.stdin.take().expect("stdin was piped");
        let writer = std::thread::spawn(move || drop(stdin.write_all(input.as_bytes())));
        let out = child.wait_with_output().map_err(io_err)?;
        let _ = writer.join();
        Ok(out)
    }

    /// Reads every blob in `wanted`, returning their contents in the order
    /// given. `wanted` is `(sha, path)`; the path is carried only so a failure
    /// can name the file.
    ///
    /// Two processes, whatever the file count — but the count that actually
    /// costs anything is network round-trips, and `cat-file --batch` alone does
    /// not batch those. Fed a list of absent blobs it spawns one `git fetch` per
    /// missing object, exactly as many round-trips as reading them one at a time
    /// (measured on git 2.55.0: 2 blobs, 2 fetches). So the shas are prefetched
    /// first with the same `fetch --stdin` call git makes internally, issued once
    /// for the whole rev; the batch then finds everything local and fetches
    /// nothing. A clone that already has the objects short-circuits without
    /// opening a connection (~4ms for ten such calls), so this is not a tax on
    /// full clones.
    ///
    /// A failed prefetch is deliberately ignored. It is an optimization, and the
    /// authoritative error belongs to the read below, which reports git's own
    /// stderr — one failure path is easier to trust than two that must agree.
    fn read_blobs(&self, wanted: &[(&str, &str)], rev: &str, prefix: &str) -> Result<Vec<Vec<u8>>> {
        if wanted.is_empty() {
            return Ok(Vec::new());
        }
        let mut input = String::with_capacity(wanted.len() * 41);
        for (sha, _) in wanted {
            input.push_str(sha);
            input.push('\n');
        }
        let _ = self.git_with_stdin(
            &[
                "-c",
                "fetch.negotiationAlgorithm=noop",
                "fetch",
                "origin",
                "--no-tags",
                "--no-write-fetch-head",
                "--recurse-submodules=no",
                "--filter=blob:none",
                "--stdin",
            ],
            input.clone(),
        );

        let out = self.git_with_stdin(&["cat-file", "--batch"], input)?;
        // This is the branch an unreachable hub takes: a lazy fetch that cannot
        // reach the promisor is fatal to the whole batch — exit 128, empty
        // stdout, no per-object record. It has to name the skill and the rev,
        // because `derive_base` calls it up to `WALK_CAP` times per skill and
        // `quay diff` runs it per skill.
        if !out.status.success() {
            return Err(QuayError::Reconcile(format!(
                "reading '{prefix}' at {rev} from the harbor failed: {} \
                 (a partial-clone blob fetch may have failed — check access to the hub)",
                String::from_utf8_lossy(&out.stderr).trim()
            )));
        }
        decode_batch(&out.stdout, wanted, rev)
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
    /// Upgrade path: `#[doc(hidden)]` hides this from the docs but not from the
    /// linker. If `quay-core` is ever published, move it behind
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

    /// A fixed number of git invocations per rev, whatever the file count:
    /// `ls-tree -r` names every path *and* every blob sha, and `read_blobs`
    /// turns that sha list into contents with one prefetch and one `cat-file
    /// --batch`. One invocation when the directory is absent, since there is then
    /// nothing to read. The `1 + 2n` this replaced spawned a process per file and,
    /// on a blobless clone, took a network round-trip per file with it.
    fn tree_at(&self, rev: &str, prefix: &str) -> Result<BTreeMap<String, Vec<u8>>> {
        // `ls-tree <rev> -- <path>` is a pathspec match, and a pathspec of
        // `skills/x` matches the directory itself, not `skills/x-tra`. The
        // trailing slash is trimmed for git's benefit here and for the keys'
        // below, where this same prefix comes back off each path.
        let prefix = prefix.trim_end_matches('/');
        // Modes are needed to drop symlinks (120000) and gitlinks (160000), so
        // this cannot use `--name-only`. `-z` makes records NUL-separated, so a
        // path containing a newline cannot split one.
        let o = git(&self.repo, &["ls-tree", "-r", "-z", rev, "--", prefix])?;
        let listing = String::from_utf8_lossy(&o.stdout);
        // (rel, sha, path): `rel` keys the map, `path` names the file if its
        // content cannot be read.
        let mut entries: Vec<(String, String, String)> = Vec::new();
        for record in listing.split('\0').filter(|s| !s.is_empty()) {
            let Some((sha, path)) = parse_ls_tree_regular_file(record) else {
                continue;
            };
            // git was asked for one pathspec, so anything outside it means the
            // listing is not what this code thinks it is. Guessing would key the
            // map by a repo-relative path, which then hashes as a different file
            // and shows up as both added upstream and deleted locally.
            let rel = path
                .strip_prefix(prefix)
                .ok_or_else(|| {
                    QuayError::Reconcile(format!("harbor listed '{path}', not under '{prefix}'"))
                })?
                .trim_start_matches('/')
                .to_string();
            if rel.is_empty() {
                continue; // `prefix` names a file, not a directory
            }
            entries.push((rel, sha, path));
        }
        let wanted: Vec<(&str, &str)> = entries
            .iter()
            .map(|(_, sha, path)| (sha.as_str(), path.as_str()))
            .collect();
        let blobs = self.read_blobs(&wanted, rev, prefix)?;
        // `read_blobs` returns one blob per request or an error, so this cannot
        // trip today. It is here because `zip` below truncates to the shorter
        // side without a word, and a listed path with no key is exactly the
        // shape this method promises never to produce: downstream it reads as
        // "you have a file the hub doesn't", and enough of them read as a skill
        // deleted upstream.
        if blobs.len() != entries.len() {
            return Err(QuayError::Reconcile(format!(
                "harbor listed {} files under '{prefix}' at {rev} but {} were read",
                entries.len(),
                blobs.len()
            )));
        }
        Ok(entries
            .into_iter()
            .map(|(rel, _, _)| rel)
            .zip(blobs)
            .collect())
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
mod decode {
    use super::*;

    /// One well-formed `cat-file --batch` record.
    fn record(sha: &str, body: &[u8]) -> Vec<u8> {
        let mut r = format!("{sha} blob {}\n", body.len()).into_bytes();
        r.extend_from_slice(body);
        r.push(b'\n');
        r
    }

    const A: &str = "1111111111111111111111111111111111111111";
    const B: &str = "2222222222222222222222222222222222222222";

    #[test]
    fn sizes_frame_the_content_so_blobs_may_contain_anything() {
        // A blob holding NULs, newlines, and a line shaped like a record header
        // is the adversarial case for any delimiter-based parse. Sizes are what
        // make it safe, and a rewrite to line-oriented reading must fail here.
        let nasty: &[u8] = b"\x00\xff\n1111111111111111111111111111111111111111 blob 99\ntail";
        let mut out = record(A, nasty);
        out.extend(record(B, b""));
        let got = decode_batch(&out, &[(A, "a.bin"), (B, "empty.md")], "HEAD").unwrap();
        assert_eq!(got, vec![nasty.to_vec(), Vec::new()]);
    }

    #[test]
    fn identical_content_asks_twice_and_is_answered_twice() {
        // Two files with the same bytes share one sha. git answers per input
        // line, not per distinct object, and `tree_at` zips the answers against
        // the listing — so deduplicating the request would silently drop a file.
        let mut out = record(A, b"same");
        out.extend(record(A, b"same"));
        let got = decode_batch(&out, &[(A, "one.md"), (A, "two.md")], "HEAD").unwrap();
        assert_eq!(got, vec![b"same".to_vec(), b"same".to_vec()]);
    }

    #[test]
    fn a_missing_record_is_a_failure_to_read_not_an_empty_file() {
        // The rule the whole module exists to protect: the sha came from the
        // tree, so `missing` is git declining to produce an object that exists.
        let out = format!("{A} missing\n").into_bytes();
        let err = decode_batch(&out, &[(A, "skills/x/SKILL.md")], "HEAD").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("skills/x/SKILL.md"), "{msg}");
        assert!(msg.contains("HEAD"), "{msg}");
        assert!(msg.contains("hub"), "names where to look: {msg}");
    }

    #[test]
    fn a_record_for_a_different_object_is_refused_rather_than_misfiled() {
        // The failure this guards is silent: without the check, `tree_at` keys
        // B's bytes under A's path and the diff is confident and wrong.
        let out = record(B, b"wrong object");
        let err = decode_batch(&out, &[(A, "a.md")], "HEAD").unwrap_err();
        assert!(err.to_string().contains("git answered"), "{err}");
    }

    #[test]
    fn truncated_and_trailing_output_are_both_refused() {
        let mut short = format!("{A} blob 10\n").into_bytes();
        short.extend_from_slice(b"only4\n");
        assert!(decode_batch(&short, &[(A, "a.md")], "HEAD").is_err());

        let mut extra = record(A, b"ok");
        extra.extend_from_slice(b"surplus\n");
        let err = decode_batch(&extra, &[(A, "a.md")], "HEAD").unwrap_err();
        assert!(err.to_string().contains("left over"), "{err}");
    }

    #[test]
    fn nothing_requested_reads_nothing() {
        assert!(decode_batch(b"", &[], "HEAD").unwrap().is_empty());
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
        fn tree_at(&self, rev: &str, prefix: &str) -> Result<BTreeMap<String, Vec<u8>>> {
            let id = if rev == "HEAD" {
                self.head_sha()?
            } else {
                rev.to_string()
            };
            let dir = format!("{}/", prefix.trim_end_matches('/'));
            Ok(self
                .blobs
                .iter()
                .filter(|((c, p), _)| *c == id && p.starts_with(&dir))
                .map(|((_, p), bytes)| (p[dir.len()..].to_string(), bytes.clone()))
                .collect())
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
