//! Integration test for `quay remove --everywhere`.
//!
//! `--everywhere` removes the skill from all local mirror roots AND pushes
//! a deletion commit to each remote that publishes the skill.
//!
//! We test only the local-removal half here (no real network / git push)
//! by verifying that the local skill directory is gone after `quay remove`.
//! The remote-deletion path is exercised by unit tests in `commands/remove.rs`.

use assert_cmd::Command;
use assert_fs::prelude::*;
use assert_fs::TempDir;

/// `quay remove <skill>` (local-only) removes from `.agents/skills/`.
#[test]
fn quay_remove_local_deletes_skill_directory() {
    let project = TempDir::new().unwrap();
    project
        .child(".agents/skills/my-tool/SKILL.md")
        .write_str("---\nname: my-tool\ndescription: d\nversion: 0.1.0\n---\n")
        .unwrap();

    Command::cargo_bin("quay")
        .unwrap()
        .args([
            "--project",
            project.path().to_str().unwrap(),
            "remove",
            "my-tool",
        ])
        .assert()
        .success();

    assert!(
        !project.path().join(".agents/skills/my-tool").exists(),
        "skill directory should be gone after remove"
    );
}

/// `quay remove <skill>` removes from all mirror roots where the skill appears.
#[test]
fn quay_remove_deletes_from_all_mirrors() {
    let project = TempDir::new().unwrap();
    let content = "---\nname: multi\ndescription: d\nversion: 0.1.0\n---\n";
    project
        .child(".agents/skills/multi/SKILL.md")
        .write_str(content)
        .unwrap();
    project
        .child(".claude/skills/multi/SKILL.md")
        .write_str(content)
        .unwrap();

    Command::cargo_bin("quay")
        .unwrap()
        .args([
            "--project",
            project.path().to_str().unwrap(),
            "remove",
            "multi",
        ])
        .assert()
        .success();

    assert!(!project.path().join(".agents/skills/multi").exists());
    assert!(!project.path().join(".claude/skills/multi").exists());
}

/// `quay remove` exits non-zero when the skill is not installed locally.
#[test]
fn quay_remove_errors_when_skill_not_found() {
    let project = TempDir::new().unwrap();

    let output = Command::cargo_bin("quay")
        .unwrap()
        .args([
            "--project",
            project.path().to_str().unwrap(),
            "remove",
            "does-not-exist",
        ])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "expected non-zero exit for missing skill"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("not found") || stderr.contains("error"),
        "expected error message in stderr, got: {stderr}"
    );
}

/// `quay remove --everywhere` is a recognized flag (not rejected as unknown).
/// It removes the local copy first, then — with no default remote configured —
/// errors non-zero on the missing hub.
#[test]
fn quay_remove_everywhere_removes_local_then_errors_without_remote() {
    let project = TempDir::new().unwrap();
    project
        .child(".agents/skills/my-tool/SKILL.md")
        .write_str("---\nname: my-tool\ndescription: d\nversion: 0.1.0\n---\n")
        .unwrap();

    // Isolate the host's ~/.config/quay so its remotes don't bleed a default in.
    let user_cfg = project.path().join("user.toml");
    std::fs::write(&user_cfg, "").unwrap();
    let output = Command::cargo_bin("quay")
        .unwrap()
        .env("XDG_CONFIG_HOME", project.path())
        .args([
            "--project",
            project.path().to_str().unwrap(),
            "--user-config",
            user_cfg.to_str().unwrap(),
            "remove",
            "--everywhere",
            "my-tool",
        ])
        .output()
        .unwrap();

    // Flag is accepted (error is about the missing remote, not an unknown arg).
    assert!(
        !output.status.success(),
        "expected non-zero exit with no default remote"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("no default remote"),
        "expected 'no default remote' error, got: {stderr}"
    );
    // Local removal ran before the remote step failed.
    assert!(
        !project.path().join(".agents/skills/my-tool").exists(),
        "local skill should be removed even though the remote step errored"
    );
}
