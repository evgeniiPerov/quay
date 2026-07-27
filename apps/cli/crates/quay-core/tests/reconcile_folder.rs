//! Folder-level local-vs-harbor comparison. Uses a real temp git harbor so the
//! listing, history walk and blob reads are all exercised end to end — the
//! single-file path already has fake-based unit tests, and the interesting
//! failures here are precisely the ones a fake would paper over.

use quay_core::reconcile::folder::{folder_report, ChangeKind};
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
fn harbor(commits: &[&[(&str, &[u8])]]) -> (TempDir, GitHarborHistory) {
    let src = tempdir().unwrap();
    let p = src.path();
    run(p, &["init", "--initial-branch=main"]);
    run(p, &["config", "user.email", "t@t.t"]);
    run(p, &["config", "user.name", "t"]);
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
    assert_eq!(changed[0].kind, ChangeKind::Modified);
}

#[test]
fn diff_is_pull_oriented_so_plus_is_what_the_hub_would_give_you() {
    use quay_core::reconcile::diff::Diff;
    let (_src, h) = harbor(&[&[("SKILL.md", b"hub line\n")]]);
    let loc = local(&[("SKILL.md", b"local line\n")]);

    let r = folder_report(loc.path(), &h, "skills/x", "1.0.0", "1.0.0").unwrap();

    let Some(Diff::Text(t)) = r.changed().next().and_then(|f| f.diff.clone()) else {
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

    let kind = |rel: &str| r.files.iter().find(|f| f.rel == rel).map(|f| f.kind);
    assert_eq!(kind("scripts/new.sh"), Some(ChangeKind::OnlyOnHub));
    assert_eq!(kind("old.md"), Some(ChangeKind::OnlyLocal));
    assert_eq!(kind("SKILL.md"), Some(ChangeKind::Same));
}

#[test]
fn a_skill_deleted_upstream_is_absent_not_merely_changed() {
    // An empty tree hashes to a real value, so this cannot come from comparing
    // hashes — and `local_edited: true` would blame the user for an upstream
    // delete.
    let (_src, h) = harbor(&[&[("SKILL.md", b"body")]]);
    let loc = local(&[("SKILL.md", b"body")]);

    let r = folder_report(loc.path(), &h, "skills/gone", "1.0.0", "1.0.0").unwrap();

    assert!(r.absent_on_head);
    assert_eq!(
        r.verdict,
        Verdict::ChangedUnknownDirection {
            local_edited: false
        }
    );
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
fn local_edit_with_an_untouched_hub_is_not_blamed_on_the_hub() {
    let (_src, h) = harbor(&[&[("SKILL.md", b"body")]]);
    let loc = local(&[("SKILL.md", b"body\nmy tweak\n")]);

    let r = folder_report(loc.path(), &h, "skills/x", "1.0.0", "1.0.0").unwrap();

    assert_eq!(
        r.verdict,
        Verdict::ChangedUnknownDirection { local_edited: true },
        "no harbor commit matches the local bytes"
    );
}
