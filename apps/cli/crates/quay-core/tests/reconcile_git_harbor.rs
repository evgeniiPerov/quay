use quay_core::reconcile::harbor_history::{GitHarborHistory, HarborHistory};
use std::process::Command;
use tempfile::tempdir;

fn run(dir: &std::path::Path, args: &[&str]) {
    let s = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .status()
        .unwrap();
    assert!(s.success(), "git {args:?} failed");
}

#[test]
fn reads_history_from_a_real_harbor() {
    // Multiple assertions are intentionally grouped in one test: building the
    // temp git repo is expensive shared setup and splitting it would duplicate
    // that cost for no benefit. The assertions cover orthogonal properties over
    // one fixture: commits_touching count, bytes_at for a present path, bytes_at
    // for an absent path (skills/none), head_sha, and is_ancestor — all are
    // independent observations of the same GitHarborHistory instance.
    let src = tempdir().unwrap();
    let p = src.path();
    run(p, &["init", "--initial-branch=main"]);
    run(p, &["config", "user.email", "t@t.t"]);
    run(p, &["config", "user.name", "t"]);
    std::fs::create_dir_all(p.join("skills/x")).unwrap();
    std::fs::write(p.join("skills/x/SKILL.md"), b"v1").unwrap();
    run(p, &["add", "-A"]);
    run(p, &["commit", "-m", "v1"]);
    std::fs::write(p.join("skills/x/SKILL.md"), b"v2").unwrap();
    run(p, &["add", "-A"]);
    run(p, &["commit", "-m", "v2"]);

    let url = format!("file://{}", p.display());
    let h = GitHarborHistory::clone_harbor(&url, None).unwrap();

    let commits = h.commits_touching("skills/x/SKILL.md").unwrap();
    assert_eq!(commits.len(), 2);
    let head_bytes = h.bytes_at("HEAD", "skills/x/SKILL.md").unwrap().unwrap();
    assert_eq!(head_bytes, b"v2");
    let older = &commits[1].id;
    let head = h.head_sha().unwrap();
    assert!(h.is_ancestor(older, &head).unwrap());
    assert!(h
        .bytes_at("HEAD", "skills/none/SKILL.md")
        .unwrap()
        .is_none());
}

#[test]
fn lists_every_file_under_a_skill_directory() {
    // A skill is a directory, so a diff has to enumerate it. `bytes_at` can
    // only answer about a path you already know.
    let src = tempdir().unwrap();
    let p = src.path();
    run(p, &["init", "--initial-branch=main"]);
    run(p, &["config", "user.email", "t@t.t"]);
    run(p, &["config", "user.name", "t"]);
    std::fs::create_dir_all(p.join("skills/x/references")).unwrap();
    std::fs::write(p.join("skills/x/SKILL.md"), b"body").unwrap();
    std::fs::write(p.join("skills/x/references/api.md"), b"api").unwrap();
    // A sibling skill must not leak into the listing.
    std::fs::create_dir_all(p.join("skills/x-tra")).unwrap();
    std::fs::write(p.join("skills/x-tra/SKILL.md"), b"other").unwrap();
    run(p, &["add", "-A"]);
    run(p, &["commit", "-m", "seed"]);

    let url = format!("file://{}", p.display());
    let h = GitHarborHistory::clone_harbor(&url, None).unwrap();

    assert_eq!(
        h.paths_at("HEAD", "skills/x").unwrap(),
        vec![
            "skills/x/SKILL.md".to_string(),
            "skills/x/references/api.md".to_string(),
        ],
        "nested files included, the `skills/x-tra` prefix match excluded"
    );
    assert!(
        h.paths_at("HEAD", "skills/gone").unwrap().is_empty(),
        "a skill absent at this rev lists nothing"
    );
}

/// Build a one-commit harbor at `p`. `allow_filter` decides whether the served
/// repo advertises `uploadpack.allowFilter`, which is what makes
/// `--filter=blob:none` actually take effect.
fn seed_harbor(p: &std::path::Path, allow_filter: bool) {
    run(p, &["init", "--initial-branch=main"]);
    run(p, &["config", "user.email", "t@t.t"]);
    run(p, &["config", "user.name", "t"]);
    // Written explicitly in both directions. `upload-pack` reads system and
    // global config too, so *omitting* the false case would let a developer or
    // self-hosted runner with `uploadpack.allowFilter = true` in ~/.gitconfig
    // turn the non-filtering fixture into a partial clone. Local config wins.
    run(
        p,
        &[
            "config",
            "uploadpack.allowFilter",
            if allow_filter { "true" } else { "false" },
        ],
    );
    std::fs::create_dir_all(p.join("skills/x/references")).unwrap();
    std::fs::write(p.join("skills/x/SKILL.md"), b"body").unwrap();
    std::fs::write(p.join("skills/x/references/api.md"), b"api").unwrap();
    run(p, &["add", "-A"]);
    run(p, &["commit", "-m", "seed"]);
}

/// The path production always takes, and which nothing exercised before this
/// test: a hub that supports filtering, so the clone is genuinely blobless and
/// every `bytes_at` is a lazy fetch over the wire rather than a local read.
///
/// Prerequisite for trusting any change to how `read_harbor` batches its reads
/// (#27) — batching a code path no test has ever run is not a refactor, it is a
/// rewrite with no safety net.
///
/// Paired with `a_non_filtering_server_falls_back_to_a_full_clone`, which
/// asserts the opposite `is_partial()` value over the same fixture shape. The
/// pairing is the point: no constant implementation of `is_partial` can satisfy
/// both, so the two tests check each other as well as the code.
#[test]
fn a_filtering_server_yields_a_genuinely_partial_clone() {
    let src = tempdir().unwrap();
    seed_harbor(src.path(), true);

    let url = format!("file://{}", src.path().display());
    let h = GitHarborHistory::clone_harbor(&url, None).unwrap();

    assert!(
        h.is_partial(),
        "uploadpack.allowFilter is set, so the clone must be a promisor clone; \
         if this fails, git ignored --filter and every partial-clone assertion \
         below is meaningless"
    );

    // Blobs are absent until asked for. These reads are the lazy fetches, and
    // they must return the same bytes a full clone would.
    assert_eq!(
        h.bytes_at("HEAD", "skills/x/SKILL.md").unwrap().unwrap(),
        b"body"
    );
    // The ordering below is load-bearing. This fixture has exactly two blobs;
    // reading SKILL.md takes the missing-object count 2 -> 1, so the clone is
    // still partial *because references/api.md has not been asked for yet*.
    // That is what proves the fetch is per-object rather than a backfill of
    // everything on first touch — the granularity #27's batching would change.
    // Move this assertion below the next read and it fails: the count reaches 0
    // and `is_partial()` goes false.
    assert!(
        h.is_partial(),
        "one blob read must not backfill the rest of the clone"
    );
    assert_eq!(
        h.bytes_at("HEAD", "skills/x/references/api.md")
            .unwrap()
            .unwrap(),
        b"api"
    );
    // Tree reads are served from the clone; only blobs are deferred.
    assert_eq!(
        h.paths_at("HEAD", "skills/x").unwrap(),
        vec![
            "skills/x/SKILL.md".to_string(),
            "skills/x/references/api.md".to_string(),
        ]
    );
    // Genuine absence, asserted while the promisor remote is still reachable:
    // `Ok(None)` here can only mean "not in the tree at this rev". The
    // unreachable case is the opposite answer, pinned by
    // `an_unreachable_promisor_errors_instead_of_reporting_an_absent_path`.
    assert!(
        h.bytes_at("HEAD", "skills/none/SKILL.md")
            .unwrap()
            .is_none(),
        "a path absent from the tree is absent, not a failed fetch"
    );
}

/// An unreachable hub must not read as "upstream deleted this skill".
///
/// On a blobless clone every unfetched blob is one network round-trip away, so
/// a token expiry or a hub outage makes `bytes_at` fail. If that failure is
/// reported as `Ok(None)`, `baseline::derive` records the empty-sha sentinel,
/// `reconcile` produces `Verdict::AbsentOnHub`, and the user is told their skill is "no
/// longer on the hub (deleted or renamed there)" — a data-loss-shaped lie about
/// what is really a transient connectivity problem.
///
/// Deleting the source repo is the cheapest faithful stand-in for that: it is
/// what git sees when the promisor remote cannot be reached.
#[test]
fn an_unreachable_promisor_errors_instead_of_reporting_an_absent_path() {
    let src = tempdir().unwrap();
    seed_harbor(src.path(), true);

    let url = format!("file://{}", src.path().display());
    let h = GitHarborHistory::clone_harbor(&url, None).unwrap();
    assert!(
        h.is_partial(),
        "the blob must still be unfetched, or killing the remote proves nothing"
    );

    // Remove the origin *before* the first read, so `skills/x/SKILL.md` has
    // never been fetched and cannot be served from the local object store.
    src.close().unwrap();

    assert!(
        h.bytes_at("HEAD", "skills/x/SKILL.md").is_err(),
        "an unreachable promisor is a failure to report, not an absent path"
    );
}

/// The path every other fixture in this file silently takes. A server without
/// `uploadpack.allowFilter` does not reject the filter — it warns and returns a
/// full clone — so this asserts the degradation is invisible to callers rather
/// than that it errors.
///
/// Note this is git ignoring the filter, not `clone_harbor` falling back: the
/// filtered clone succeeded. The fallback branch is a separate, untested path.
///
/// Paired with `a_filtering_server_yields_a_genuinely_partial_clone`: the two
/// assert opposite `is_partial()` values over the same fixture shape, so no
/// constant implementation of `is_partial` can satisfy both.
#[test]
fn a_non_filtering_server_falls_back_to_a_full_clone() {
    let src = tempdir().unwrap();
    seed_harbor(src.path(), false);

    let url = format!("file://{}", src.path().display());
    let h = GitHarborHistory::clone_harbor(&url, None).unwrap();

    assert!(
        !h.is_partial(),
        "without uploadpack.allowFilter git must hand back a full clone"
    );
    assert_eq!(
        h.bytes_at("HEAD", "skills/x/SKILL.md").unwrap().unwrap(),
        b"body"
    );
    assert_eq!(
        h.paths_at("HEAD", "skills/x").unwrap(),
        vec![
            "skills/x/SKILL.md".to_string(),
            "skills/x/references/api.md".to_string(),
        ]
    );
}
