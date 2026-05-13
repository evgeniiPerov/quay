//! Integration tests for `SkillPusher` with `direct_branch` using real git.
//!
//! These tests set up a bare repository and a working clone, then run the
//! full push pipeline, asserting that the commit lands on the expected branch.

use quay_core::provider::FakeOpener;
use quay_core::{BumpKind, Config, GitShellClient, PushMode, RemoteConfig, SkillPusher};
use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

// ── git helpers ───────────────────────────────────────────────────────────────

fn git_in(args: &[&str], dir: &Path) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn git: {e}"));
    assert!(
        out.status.success(),
        "git {:?} in {} failed:\n{}",
        args,
        dir.display(),
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Initialize a bare repo with a single `main` commit.
fn init_bare_with_main(bare: &Path) {
    // Force the default branch to `main` regardless of the host's
    // `init.defaultBranch` config; without this the bare repo's HEAD
    // points at a non-existent ref and clones fail to check anything out.
    Command::new("git")
        .arg("init")
        .arg("--bare")
        .arg("--initial-branch=main")
        .arg(bare)
        .output()
        .unwrap();

    let work = tempfile::tempdir().unwrap();
    Command::new("git")
        .arg("clone")
        .arg(bare)
        .arg(work.path())
        .output()
        .unwrap();

    std::fs::write(work.path().join("README.md"), b"hub\n").unwrap();

    // Set identity in the working clone (avoids global config dependency).
    git_in(&["config", "user.email", "t@e"], work.path());
    git_in(&["config", "user.name", "T"], work.path());
    git_in(&["checkout", "-B", "main"], work.path());
    git_in(&["add", "-A"], work.path());
    git_in(&["commit", "-m", "init"], work.path());
    git_in(&["push", "-u", "origin", "main"], work.path());
}

/// Initialize a bare repo that also has a `develop` branch (branched from `main`).
fn init_bare_with_develop(bare: &Path) {
    init_bare_with_main(bare);

    let work = tempfile::tempdir().unwrap();
    Command::new("git")
        .arg("clone")
        .arg(bare)
        .arg(work.path())
        .output()
        .unwrap();
    git_in(&["config", "user.email", "t@e"], work.path());
    git_in(&["config", "user.name", "T"], work.path());
    // Create develop from main.
    git_in(&["checkout", "-b", "develop"], work.path());
    git_in(&["push", "-u", "origin", "develop"], work.path());
}

fn branch_exists_in_bare(bare: &Path, branch: &str) -> bool {
    let out = Command::new("git")
        .arg("ls-remote")
        .arg(bare)
        .arg(format!("refs/heads/{}", branch))
        .output()
        .unwrap();
    let s = String::from_utf8_lossy(&out.stdout);
    s.contains(&format!("refs/heads/{}", branch))
}

fn log_on_branch(bare: &Path, branch: &str) -> String {
    let work = tempfile::tempdir().unwrap();
    Command::new("git")
        .arg("clone")
        .arg("--branch")
        .arg(branch)
        .arg(bare)
        .arg(work.path())
        .output()
        .unwrap();
    let out = Command::new("git")
        .arg("-C")
        .arg(work.path())
        .arg("log")
        .arg("--oneline")
        .arg("-5")
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).to_string()
}

// ── config / pusher helpers ───────────────────────────────────────────────────

fn make_config(bare: &Path, push_mode: PushMode, direct_branch: Option<String>) -> Config {
    let mut remotes = BTreeMap::new();
    remotes.insert(
        "hub".into(),
        RemoteConfig {
            url: bare.to_str().unwrap().to_string(),
            default: true,
            provider: None,
            push_mode,
            direct_branch,
        },
    );
    Config {
        user: quay_core::config::UserSection {
            name: Some("Test User".into()),
            email: Some("test@example.com".into()),
        },
        remotes,
        install: Default::default(),
    }
}

fn make_local_skill(project: &Path, name: &str) {
    let dir = project.join(".agents/skills").join(name);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("SKILL.md"),
        "---\nname: my-skill\ndescription: desc\nversion: 0.1.0\n---\nbody\n",
    )
    .unwrap();
}

// ── tests ─────────────────────────────────────────────────────────────────────

/// Direct push with `direct_branch = "develop"` lands on an existing `develop` branch.
#[test]
fn direct_push_lands_on_existing_develop_branch() {
    let tmp = tempfile::TempDir::new().unwrap();
    let bare = tmp.path().join("hub.git");
    init_bare_with_develop(&bare);

    let project = tmp.path().join("project");
    make_local_skill(&project, "my-skill");

    let cfg = make_config(&bare, PushMode::Direct, Some("develop".into()));
    let git = GitShellClient;
    let opener = FakeOpener;
    let clone_root = tmp.path().join("clones");
    std::fs::create_dir_all(&clone_root).unwrap();

    let pusher = SkillPusher {
        config: &cfg,
        git: &git,
        opener: &opener,
        project_root: project,
        config_dir: None,
        author: Some(("Test".into(), "test@example.com".into())),
    };

    let result = pusher
        .push(
            "my-skill",
            None,
            BumpKind::AsWritten,
            &clone_root,
            None,
            None, // no override — config's direct_branch = "develop" applies
        )
        .unwrap();

    assert_eq!(result.branch, "develop");
    assert!(
        branch_exists_in_bare(&bare, "develop"),
        "develop branch must exist in bare repo"
    );
    let log = log_on_branch(&bare, "develop");
    assert!(
        log.contains("my-skill"),
        "commit must reference skill name; log:\n{}",
        log
    );
}

/// When `direct_branch` points to a branch that does NOT exist yet, it is
/// auto-created from the default branch and the push creates it on the remote.
#[test]
fn direct_push_auto_creates_missing_branch() {
    let tmp = tempfile::TempDir::new().unwrap();
    let bare = tmp.path().join("hub.git");
    // Hub only has main — no `newbranch` yet.
    init_bare_with_main(&bare);

    let project = tmp.path().join("project");
    make_local_skill(&project, "my-skill");

    assert!(
        !branch_exists_in_bare(&bare, "newbranch"),
        "newbranch must not exist initially"
    );

    let cfg = make_config(&bare, PushMode::Direct, Some("newbranch".into()));
    let git = GitShellClient;
    let opener = FakeOpener;
    let clone_root = tmp.path().join("clones");
    std::fs::create_dir_all(&clone_root).unwrap();

    let pusher = SkillPusher {
        config: &cfg,
        git: &git,
        opener: &opener,
        project_root: project,
        config_dir: None,
        author: Some(("Test".into(), "test@example.com".into())),
    };

    let result = pusher
        .push(
            "my-skill",
            None,
            BumpKind::AsWritten,
            &clone_root,
            None,
            None,
        )
        .unwrap();

    assert_eq!(result.branch, "newbranch");
    assert!(
        branch_exists_in_bare(&bare, "newbranch"),
        "newbranch must have been auto-created in the bare repo"
    );
    let log = log_on_branch(&bare, "newbranch");
    assert!(
        log.contains("my-skill"),
        "commit must be on newbranch; log:\n{}",
        log
    );
}

/// Per-invocation `direct_branch_override` wins over `direct_branch = None` in config.
#[test]
fn per_invocation_direct_branch_override_creates_branch() {
    let tmp = tempfile::TempDir::new().unwrap();
    let bare = tmp.path().join("hub.git");
    init_bare_with_main(&bare);

    let project = tmp.path().join("project");
    make_local_skill(&project, "my-skill");

    // Config says no direct_branch (would default to main).
    let cfg = make_config(&bare, PushMode::Direct, None);
    let git = GitShellClient;
    let opener = FakeOpener;
    let clone_root = tmp.path().join("clones");
    std::fs::create_dir_all(&clone_root).unwrap();

    let pusher = SkillPusher {
        config: &cfg,
        git: &git,
        opener: &opener,
        project_root: project,
        config_dir: None,
        author: Some(("Test".into(), "test@example.com".into())),
    };

    let result = pusher
        .push(
            "my-skill",
            None,
            BumpKind::AsWritten,
            &clone_root,
            None,
            Some("skills"), // per-invocation override
        )
        .unwrap();

    assert_eq!(result.branch, "skills");
    assert!(
        branch_exists_in_bare(&bare, "skills"),
        "skills branch must exist after push"
    );
}

/// Regression: when remote `develop` has commits AHEAD of `main`, a direct
/// push targeting `develop` must fast-forward — not reject with non-FF. The
/// pusher must clone `develop` (so HEAD tracks `origin/develop`), not clone
/// `main` and branch from it.
#[test]
fn direct_push_to_diverged_develop_fast_forwards() {
    let tmp = tempfile::TempDir::new().unwrap();
    let bare = tmp.path().join("hub.git");
    init_bare_with_develop(&bare);

    // Add a commit to `develop` on the remote so it diverges from `main`.
    let work = tempfile::tempdir().unwrap();
    Command::new("git")
        .arg("clone")
        .arg("--branch")
        .arg("develop")
        .arg(&bare)
        .arg(work.path())
        .output()
        .unwrap();
    git_in(&["config", "user.email", "t@e"], work.path());
    git_in(&["config", "user.name", "T"], work.path());
    std::fs::write(work.path().join("EXTRA.md"), b"extra\n").unwrap();
    git_in(&["add", "-A"], work.path());
    git_in(&["commit", "-m", "diverge"], work.path());
    git_in(&["push", "origin", "develop"], work.path());

    let project = tmp.path().join("project");
    make_local_skill(&project, "my-skill");

    let cfg = make_config(&bare, PushMode::Direct, Some("develop".into()));
    let git = GitShellClient;
    let opener = FakeOpener;
    let clone_root = tmp.path().join("clones");
    std::fs::create_dir_all(&clone_root).unwrap();

    let pusher = SkillPusher {
        config: &cfg,
        git: &git,
        opener: &opener,
        project_root: project,
        config_dir: None,
        author: Some(("Test".into(), "test@example.com".into())),
    };

    let result = pusher
        .push(
            "my-skill",
            None,
            BumpKind::AsWritten,
            &clone_root,
            None,
            None,
        )
        .unwrap();

    assert_eq!(result.branch, "develop");
    let log = log_on_branch(&bare, "develop");
    assert!(
        log.contains("diverge"),
        "develop must still contain the prior diverge commit; log:\n{log}"
    );
    assert!(
        log.contains("my-skill"),
        "develop must now also contain the skill commit; log:\n{log}"
    );
}
