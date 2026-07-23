//! `quay agents` — registry-driven mirroring into coding-agent skill dirs.

use assert_cmd::Command;
use assert_fs::prelude::*;

fn quay(project: &assert_fs::TempDir) -> Command {
    let mut cmd = Command::cargo_bin("quay").unwrap();
    cmd.args(["--project", project.path().to_str().unwrap()]);
    cmd
}

fn project_with_skill() -> assert_fs::TempDir {
    let p = assert_fs::TempDir::new().unwrap();
    p.child(".agents/skills/demo/SKILL.md")
        .write_str("---\nname: demo\n---\n")
        .unwrap();
    p
}

#[test]
fn list_shows_known_agents() {
    let p = assert_fs::TempDir::new().unwrap();
    quay(&p)
        .args(["agents", "list"])
        .assert()
        .success()
        .stdout(predicates::str::contains("claude-code"))
        .stdout(predicates::str::contains("Claude Code"));
}

#[test]
fn link_creates_mirror_symlink_to_canonical() {
    let p = project_with_skill();
    quay(&p)
        .args(["agents", "link", "--agent", "claude-code"])
        .assert()
        .success()
        .stdout(predicates::str::contains("created"));

    let mirror = p.path().join(".claude/skills/demo");
    assert!(mirror.exists(), "mirror should exist");
    assert!(
        std::fs::symlink_metadata(&mirror)
            .unwrap()
            .file_type()
            .is_symlink(),
        "mirror should be a symlink"
    );
}

#[test]
fn link_is_idempotent() {
    let p = project_with_skill();
    quay(&p)
        .args(["agents", "link", "-a", "claude-code"])
        .assert()
        .success();
    quay(&p)
        .args(["agents", "link", "-a", "claude-code"])
        .assert()
        .success()
        .stdout(predicates::str::contains("ok"));
}

#[test]
fn unknown_agent_errors() {
    let p = project_with_skill();
    quay(&p)
        .args(["agents", "link", "--agent", "not-a-real-agent"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("unknown agent"));
}

#[test]
fn universal_agent_emits_no_mirror() {
    let p = project_with_skill();
    // codex reads `.agents/skills` directly — nothing to mirror.
    quay(&p)
        .args(["agents", "link", "--agent", "codex"])
        .assert()
        .success()
        .stdout(predicates::str::contains("nothing to mirror"));
    assert!(!p.path().join(".codex").exists());
}

#[test]
fn project_scope_persists_mirror_into_config() {
    let p = project_with_skill();
    // initialize so persistence kicks in
    p.child(".quay/config.toml").write_str("").unwrap();
    quay(&p)
        .args(["agents", "link", "-a", "claude-code"])
        .assert()
        .success();

    let cfg = std::fs::read_to_string(p.path().join(".quay/config.toml")).unwrap();
    assert!(
        cfg.contains(".claude/skills"),
        "mirror should be recorded in config: {cfg}"
    );
}
