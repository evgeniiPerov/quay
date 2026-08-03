//! Folder-level local-vs-harbor comparison. Uses a real temp git harbor so the
//! listing, history walk and blob reads are all exercised end to end — the
//! single-file path already has fake-based unit tests, and the interesting
//! failures here are precisely the ones a fake would paper over.

use quay_core::reconcile::folder::{folder_report, Change};
use quay_core::reconcile::harbor_history::GitHarborHistory;
use quay_core::reconcile::verdict::Verdict;
use std::path::Path;
use std::process::Command;
use tempfile::{tempdir, TempDir};

fn run(dir: &Path, args: &[&str]) {
    let s = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .status()
        .unwrap();
    assert!(s.success(), "git {args:?} failed");
}

/// A harbor whose `skills/x` directory takes each state in `commits` in turn,
/// one commit per state. Each state is a list of (path relative to the skill
/// dir, contents).
///
/// The served repo advertises `uploadpack.allowFilter`, so every test in this
/// file runs against a genuinely blobless clone — the path production always
/// takes against GitHub. Written explicitly rather than left to the default:
/// `upload-pack` reads global config too, so omitting it would make which path
/// these tests exercise depend on the developer's `~/.gitconfig`.
fn harbor(commits: &[&[(&str, &[u8])]]) -> (TempDir, GitHarborHistory) {
    let src = tempdir().unwrap();
    let p = src.path();
    run(p, &["init", "--initial-branch=main"]);
    run(p, &["config", "user.email", "t@t.t"]);
    run(p, &["config", "user.name", "t"]);
    run(p, &["config", "uploadpack.allowFilter", "true"]);
    for (i, state) in commits.iter().enumerate() {
        let dir = p.join("skills/x");
        let _ = std::fs::remove_dir_all(&dir);
        for (rel, body) in state.iter() {
            let full = dir.join(rel);
            std::fs::create_dir_all(full.parent().unwrap()).unwrap();
            std::fs::write(full, body).unwrap();
        }
        run(p, &["add", "-A"]);
        run(p, &["commit", "--allow-empty", "-m", &format!("state {i}")]);
    }
    let url = format!("file://{}", p.display());
    let h = GitHarborHistory::clone_harbor(&url, None).unwrap();
    (src, h)
}

/// A local skill directory holding `files`.
fn local(files: &[(&str, &[u8])]) -> TempDir {
    let dir = tempdir().unwrap();
    for (rel, body) in files {
        let full = dir.path().join(rel);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(full, body).unwrap();
    }
    dir
}

#[test]
fn a_sibling_file_edited_on_hub_is_reported_even_though_skill_md_matches() {
    // The case the single-file reconcile calls `Identical`.
    let (_src, h) = harbor(&[
        &[("SKILL.md", b"body"), ("references/api.md", b"GET /v1")],
        &[("SKILL.md", b"body"), ("references/api.md", b"GET /v2")],
    ]);
    let loc = local(&[("SKILL.md", b"body"), ("references/api.md", b"GET /v1")]);

    let r = folder_report(loc.path(), &h, "skills/x", "1.0.0", "1.0.0").unwrap();

    assert!(
        matches!(r.verdict, Verdict::HubNewer { .. }),
        "harbor moved forward and the local copy matches an earlier commit, \
         so the direction is knowable: {:?}",
        r.verdict
    );
    let changed: Vec<_> = r.changed().collect();
    assert_eq!(changed.len(), 1, "only api.md moved");
    assert_eq!(changed[0].rel, "references/api.md");
    assert!(
        matches!(changed[0].change, Change::Modified(_)),
        "{:?}",
        changed[0].change
    );
}

#[test]
fn diff_is_pull_oriented_so_plus_is_what_the_hub_would_give_you() {
    use quay_core::reconcile::diff::Diff;
    let (_src, h) = harbor(&[&[("SKILL.md", b"hub line\n")]]);
    let loc = local(&[("SKILL.md", b"local line\n")]);

    let r = folder_report(loc.path(), &h, "skills/x", "1.0.0", "1.0.0").unwrap();

    let Some(Diff::Text(t)) = r.changed().next().and_then(|f| f.change.diff()) else {
        panic!("expected a text diff");
    };
    assert!(
        t.contains("+hub line\n"),
        "the hub's content is the addition: {t}"
    );
    assert!(
        t.contains("-local line\n"),
        "your content is what it replaces: {t}"
    );
}

#[test]
fn identical_folders_report_identical() {
    let (_src, h) = harbor(&[&[("SKILL.md", b"body"), ("scripts/run.sh", b"echo")]]);
    let loc = local(&[("SKILL.md", b"body"), ("scripts/run.sh", b"echo")]);

    let r = folder_report(loc.path(), &h, "skills/x", "1.0.0", "1.0.0").unwrap();

    assert_eq!(r.verdict, Verdict::Identical);
    assert_eq!(r.changed().count(), 0);
    assert_eq!(r.local_hash, r.head_hash);
}

#[test]
fn files_added_and_removed_on_hub_get_their_own_kinds() {
    let (_src, h) = harbor(&[&[("SKILL.md", b"body"), ("scripts/new.sh", b"echo hi")]]);
    let loc = local(&[("SKILL.md", b"body"), ("old.md", b"stale")]);

    let r = folder_report(loc.path(), &h, "skills/x", "1.0.0", "1.0.0").unwrap();

    let change = |rel: &str| r.files.iter().find(|f| f.rel == rel).map(|f| &f.change);
    assert!(
        matches!(change("scripts/new.sh"), Some(Change::OnlyOnHub(_))),
        "{:?}",
        change("scripts/new.sh")
    );
    assert!(
        matches!(change("old.md"), Some(Change::OnlyLocal(_))),
        "{:?}",
        change("old.md")
    );
    assert!(
        matches!(change("SKILL.md"), Some(Change::Same)),
        "{:?}",
        change("SKILL.md")
    );
}

#[test]
fn a_skill_deleted_upstream_is_absent_not_merely_changed() {
    // An empty tree hashes to a real value, so this cannot come from comparing
    // hashes — and `ChangedUnknownDirection` would leave the caller free to
    // blame the user for an upstream delete.
    let (_src, h) = harbor(&[&[("SKILL.md", b"body")]]);
    let loc = local(&[("SKILL.md", b"body")]);

    let r = folder_report(loc.path(), &h, "skills/gone", "1.0.0", "1.0.0").unwrap();

    assert_eq!(r.verdict, Verdict::AbsentOnHub);
    assert!(r.absent_on_hub());
}

#[test]
fn a_hub_dotfile_is_not_permanent_drift() {
    // Dotfiles are never pushed to an install, so counting one on the hub side
    // would report a difference that no `quay add` could ever resolve.
    let (_src, h) = harbor(&[&[("SKILL.md", b"body"), (".gitkeep", b"")]]);
    let loc = local(&[("SKILL.md", b"body")]);

    let r = folder_report(loc.path(), &h, "skills/x", "1.0.0", "1.0.0").unwrap();

    assert_eq!(r.verdict, Verdict::Identical, "{:?}", r.files);
}

#[test]
fn crlf_line_endings_alone_are_not_a_difference() {
    // git's default core.autocrlf on Windows hands back CRLF at checkout, while
    // the hub's blobs hold LF. Comparing raw bytes would mark every file in
    // every skill as Modified on that platform.
    let (_src, h) = harbor(&[&[("SKILL.md", b"a\nb\n"), ("refs/x.md", b"c\nd\n")]]);
    let loc = local(&[("SKILL.md", b"a\r\nb\r\n"), ("refs/x.md", b"c\r\nd\r\n")]);

    let r = folder_report(loc.path(), &h, "skills/x", "1.0.0", "1.0.0").unwrap();

    assert_eq!(r.verdict, Verdict::Identical, "{:?}", r.files);
    assert_eq!(r.changed().count(), 0);
}

#[test]
fn binary_byte_counts_are_attributed_to_the_right_side() {
    use quay_core::reconcile::diff::Diff;
    // 4 bytes on the hub, 2 locally. `render`'s arguments are passed
    // local-then-hub to get pull-oriented diff signs, so a struct whose fields
    // are named by side would silently receive them backwards.
    let (_src, h) = harbor(&[&[
        ("SKILL.md", b"body"),
        ("logo.png", &[0xff, 0xfe, 0x00, 0x01]),
    ]]);
    let loc = local(&[("SKILL.md", b"body"), ("logo.png", &[0xff, 0xfe])]);

    let r = folder_report(loc.path(), &h, "skills/x", "1.0.0", "1.0.0").unwrap();
    let f = r.changed().find(|f| f.rel == "logo.png").expect("listed");

    assert_eq!(
        f.change.diff(),
        // folder_report renders render(local, hub): old = local, new = hub.
        Some(&Diff::Binary {
            old_bytes: 2,
            new_bytes: 4
        }),
        "byte counts must follow their argument position"
    );
}

#[test]
fn a_hub_symlink_is_not_permanent_unresolvable_drift() {
    // `read_local` skips symlinks (a link must not pull in out-of-tree files),
    // so counting one on the hub side makes the skill differ forever: `quay add
    // --force` skips it too, and the user gets blamed for an edit they cannot
    // undo.
    let src = tempdir().unwrap();
    let p = src.path();
    run(p, &["init", "--initial-branch=main"]);
    run(p, &["config", "user.email", "t@t.t"]);
    run(p, &["config", "user.name", "t"]);
    std::fs::create_dir_all(p.join("skills/x")).unwrap();
    std::fs::write(p.join("skills/x/SKILL.md"), b"body").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink("SKILL.md", p.join("skills/x/alias.md")).unwrap();
    run(p, &["add", "-A"]);
    run(p, &["commit", "-m", "seed"]);
    let h = GitHarborHistory::clone_harbor(&format!("file://{}", p.display()), None).unwrap();

    let loc = local(&[("SKILL.md", b"body")]);
    let r = folder_report(loc.path(), &h, "skills/x", "1.0.0", "1.0.0").unwrap();

    assert_eq!(r.verdict, Verdict::Identical, "{:?}", r.files);
}

#[test]
fn exhausting_the_history_walk_does_not_claim_the_user_edited_the_skill() {
    // Past WALK_CAP a matching commit exists but is never reached. Saying "no
    // commit matches your copy" is then a false statement of fact, and blames
    // the user for an edit they never made. The verdict cannot carry that
    // distinction — `base_search_truncated` is what forbids the claim.
    let src = tempdir().unwrap();
    let p = src.path();
    run(p, &["init", "--initial-branch=main"]);
    run(p, &["config", "user.email", "t@t.t"]);
    run(p, &["config", "user.name", "t"]);
    std::fs::create_dir_all(p.join("skills/x")).unwrap();
    for i in 0..60 {
        std::fs::write(p.join("skills/x/SKILL.md"), format!("body {i}\n")).unwrap();
        run(p, &["add", "-A"]);
        run(p, &["commit", "-q", "-m", &format!("rev {i}")]);
    }
    let h = GitHarborHistory::clone_harbor(&format!("file://{}", p.display()), None).unwrap();

    // Matches the oldest commit, well past the cap.
    let loc = local(&[("SKILL.md", b"body 0\n")]);
    let r = folder_report(loc.path(), &h, "skills/x", "1.0.0", "1.0.0").unwrap();

    assert!(
        r.base_search_truncated,
        "the walk hit its cap, so 'no base found' is not a conclusion"
    );
    assert_eq!(
        r.verdict,
        Verdict::ChangedUnknownDirection,
        "the direction is unknown either way; only the flag says why"
    );
}

#[test]
fn a_hub_directory_holding_only_dotfiles_is_not_deleted_upstream() {
    // `AbsentOnHub` drives the headline "no longer on the hub". Deriving it
    // from the post-filter map means a hub carrying only a `.gitkeep` reports a
    // live skill as deleted.
    let (_src, h) = harbor(&[&[(".gitkeep", b"")]]);
    let loc = local(&[("SKILL.md", b"body")]);

    let r = folder_report(loc.path(), &h, "skills/x", "1.0.0", "1.0.0").unwrap();

    assert!(
        !r.absent_on_hub(),
        "the directory exists on the hub; it just holds nothing installable"
    );
}

#[test]
fn an_unreachable_hub_is_an_error_not_a_deleted_skill() {
    // The whole composition, not just `tree_at`: the folder report must fail
    // rather than reach `Verdict::AbsentOnHub`, which the CLI renders as "no
    // longer on the hub (deleted or renamed there)". Trees survive in the local
    // clone, so the listing still succeeds and only the blob reads fail — which
    // is precisely how a token expiry looks, and why absence has to be a fact
    // about the tree rather than about a read.
    let (src, h) = harbor(&[&[("SKILL.md", b"body")]]);
    let loc = local(&[("SKILL.md", b"body")]);
    src.close().unwrap(); // hub gone before any blob was fetched

    let err = folder_report(loc.path(), &h, "skills/x", "1.0.0", "1.0.0")
        .expect_err("an unreachable hub is a failure to report");

    assert!(
        err.to_string().contains("skills/x"),
        "the error names the skill it could not read: {err}"
    );
}

#[test]
fn local_edit_with_an_untouched_hub_is_not_blamed_on_the_hub() {
    let (_src, h) = harbor(&[&[("SKILL.md", b"body")]]);
    let loc = local(&[("SKILL.md", b"body\nmy tweak\n")]);

    let r = folder_report(loc.path(), &h, "skills/x", "1.0.0", "1.0.0").unwrap();

    assert_eq!(
        r.verdict,
        Verdict::ChangedUnknownDirection,
        "no harbor commit matches the local bytes"
    );
    assert!(
        !r.base_search_truncated,
        "the whole history was searched, so 'no base' is a conclusion here"
    );
}
