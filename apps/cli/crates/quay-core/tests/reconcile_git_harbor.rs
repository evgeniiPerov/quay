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
