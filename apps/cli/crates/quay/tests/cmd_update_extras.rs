//! `quay update` / `quay add --force` and local files the new version lacks.
//!
//! A local bare git "harbor" (no network) serves registry.json plus a skill.
//! The test installs v1, drops an extra file into the install, publishes v2,
//! then asserts what each flag does to that extra file.

use assert_cmd::Command;
use predicates::prelude::*;
use std::path::Path;
use std::process;
use tempfile::TempDir;

fn git(dir: &Path, args: &[&str]) {
    let status = process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .status()
        .expect("git");
    assert!(status.success(), "git {args:?} failed");
}

fn registry_json(version: &str) -> String {
    format!(
        r#"{{
    "hub": "test-harbor",
    "generated_at": "2026-05-01T00:00:00Z",
    "schema_version": 1,
    "skills": {{
        "foo": {{
            "version": "{version}",
            "description": "Foo skill",
            "tags": [],
            "path": "skills/foo",
            "sha": "deadbeef",
            "files": ["SKILL.md"]
        }}
    }}
}}"#
    )
}

fn skill_body(version: &str) -> String {
    format!("---\nname: foo\ndescription: Foo skill\nversion: {version}\n---\nbody {version}\n")
}

/// Bare harbor holding `foo` at 1.0.0. Returns (work, bare) — both must stay
/// alive for the length of the test.
fn make_harbor() -> (TempDir, TempDir) {
    let work = TempDir::new().unwrap();
    let bare = TempDir::new().unwrap();

    git(bare.path(), &["init", "--bare", "--initial-branch=main"]);
    git(work.path(), &["init", "--initial-branch=main"]);
    git(
        work.path(),
        &["config", "user.email", "quay-test@example.com"],
    );
    git(work.path(), &["config", "user.name", "quay-test"]);

    std::fs::write(work.path().join("registry.json"), registry_json("1.0.0")).unwrap();
    let skill_dir = work.path().join("skills/foo");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(skill_dir.join("SKILL.md"), skill_body("1.0.0")).unwrap();

    git(work.path(), &["add", "-A"]);
    git(work.path(), &["commit", "-m", "init harbor"]);
    let bare_url = bare.path().to_str().unwrap().to_string();
    git(work.path(), &["remote", "add", "origin", &bare_url]);
    git(work.path(), &["push", "origin", "main:main"]);

    (work, bare)
}

/// Publish 2.0.0 so `quay update` has something to do.
fn publish_v2(work: &TempDir) {
    std::fs::write(work.path().join("registry.json"), registry_json("2.0.0")).unwrap();
    std::fs::write(work.path().join("skills/foo/SKILL.md"), skill_body("2.0.0")).unwrap();
    git(work.path(), &["add", "-A"]);
    git(work.path(), &["commit", "-m", "v2"]);
    git(work.path(), &["push", "origin", "main:main"]);
}

/// Single-quoted TOML literal: on Windows the url carries `C:\Users\...`, and
/// inside a basic string `\U` is read as a unicode escape.
fn project_config_for_url(url: &str) -> String {
    format!("[remotes.hub]\nurl = '{url}'\ndefault = true\n")
}

fn quay() -> Command {
    Command::cargo_bin("quay").unwrap()
}

/// Install foo@1.0.0 into a fresh project, drop `notes.md` beside it, publish
/// v2. Returns (project, work, bare, config_home) — all must stay alive.
fn project_ready_to_update() -> (TempDir, TempDir, TempDir, TempDir) {
    let (work, bare) = make_harbor();
    let project = TempDir::new().unwrap();
    let config_home = TempDir::new().unwrap();
    let p = project.path().to_str().unwrap().to_string();

    std::fs::create_dir_all(project.path().join(".quay")).unwrap();
    std::fs::write(
        project.path().join(".quay/config.toml"),
        project_config_for_url(bare.path().to_str().unwrap()),
    )
    .unwrap();

    quay()
        .env("XDG_CONFIG_HOME", config_home.path())
        .args(["--project", &p, "add", "foo"])
        .assert()
        .success();

    std::fs::write(
        project.path().join(".agents/skills/foo/notes.md"),
        b"my notes",
    )
    .unwrap();

    publish_v2(&work);
    (project, work, bare, config_home)
}

#[test]
fn update_without_a_flag_keeps_extras_and_notes_them() {
    let (project, _work, _bare, config_home) = project_ready_to_update();
    let p = project.path().to_str().unwrap().to_string();

    quay()
        .env("XDG_CONFIG_HOME", config_home.path())
        .args(["--project", &p, "update", "foo"])
        .assert()
        .success()
        .stderr(predicates::str::contains("kept 1 files"))
        .stderr(predicates::str::contains("notes.md"))
        .stderr(predicates::str::contains("--delete-extra"));

    assert!(project.path().join(".agents/skills/foo/notes.md").exists());
}

#[test]
fn update_delete_extra_removes_them() {
    let (project, _work, _bare, config_home) = project_ready_to_update();
    let p = project.path().to_str().unwrap().to_string();

    quay()
        .env("XDG_CONFIG_HOME", config_home.path())
        .args(["--project", &p, "update", "foo", "--delete-extra"])
        .assert()
        .success();

    assert!(
        !project.path().join(".agents/skills/foo/notes.md").exists(),
        "--delete-extra must remove the extra file"
    );
    assert!(project.path().join(".agents/skills/foo/SKILL.md").exists());
}

#[test]
fn update_keep_extra_is_silent() {
    let (project, _work, _bare, config_home) = project_ready_to_update();
    let p = project.path().to_str().unwrap().to_string();

    quay()
        .env("XDG_CONFIG_HOME", config_home.path())
        .args(["--project", &p, "update", "foo", "--keep-extra"])
        .assert()
        .success()
        .stderr(predicates::str::contains("kept 1 files").not());

    assert!(project.path().join(".agents/skills/foo/notes.md").exists());
}

#[test]
fn update_json_keeps_and_leaves_stdout_parseable() {
    let (project, _work, _bare, config_home) = project_ready_to_update();
    let p = project.path().to_str().unwrap().to_string();

    let out = quay()
        .env("XDG_CONFIG_HOME", config_home.path())
        .args(["--project", &p, "update", "foo", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    serde_json::from_slice::<serde_json::Value>(&out).expect("stdout must stay valid JSON");
    assert!(project.path().join(".agents/skills/foo/notes.md").exists());
}

#[test]
fn keep_and_delete_extra_conflict() {
    let (project, _work, _bare, config_home) = project_ready_to_update();
    let p = project.path().to_str().unwrap().to_string();

    quay()
        .env("XDG_CONFIG_HOME", config_home.path())
        .args([
            "--project",
            &p,
            "update",
            "foo",
            "--keep-extra",
            "--delete-extra",
        ])
        .assert()
        .failure()
        .stderr(predicates::str::contains("cannot be used with"));
}

#[test]
fn add_force_delete_extra_takes_the_same_path() {
    let (project, _work, _bare, config_home) = project_ready_to_update();
    let p = project.path().to_str().unwrap().to_string();

    quay()
        .env("XDG_CONFIG_HOME", config_home.path())
        .args(["--project", &p, "add", "foo", "--force", "--delete-extra"])
        .assert()
        .success();

    assert!(
        !project.path().join(".agents/skills/foo/notes.md").exists(),
        "add --force must honour --delete-extra"
    );
}

#[test]
fn add_force_without_a_flag_keeps_extras() {
    let (project, _work, _bare, config_home) = project_ready_to_update();
    let p = project.path().to_str().unwrap().to_string();

    quay()
        .env("XDG_CONFIG_HOME", config_home.path())
        .args(["--project", &p, "add", "foo", "--force"])
        .assert()
        .success();

    assert!(project.path().join(".agents/skills/foo/notes.md").exists());
}
