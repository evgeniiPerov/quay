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
    registry_json_with(version, &["SKILL.md"])
}

fn registry_json_with(version: &str, files: &[&str]) -> String {
    let files = serde_json::to_string(files).unwrap();
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
            "files": {files}
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

/// Fails unless the install holds v2's `SKILL.md`. File survival alone passes
/// just as well for an update that decided everything and then never swapped.
fn assert_v2_landed(project: &TempDir) {
    let installed =
        std::fs::read_to_string(project.path().join(".agents/skills/foo/SKILL.md")).unwrap();
    assert!(
        installed.contains("body 2.0.0"),
        "the update must actually have landed v2: {installed}"
    );
}

fn quay() -> Command {
    Command::cargo_bin("quay").unwrap()
}

/// Install foo@1.0.0 into a fresh project, drop `notes.md` beside it, publish
/// v2. Returns (project, work, bare, config_home) — all must stay alive.
fn project_ready_to_update() -> (TempDir, TempDir, TempDir, TempDir) {
    project_ready_to_update_with("")
}

/// A copy-strategy mirror: the one kind that holds its own bytes, and so the
/// one an update can leave stale.
const COPY_MIRROR: &str =
    "\n[install]\nmirrors = [{ path = \".cursor/rules\", strategy = \"copy\" }]\n";

/// [`project_ready_to_update`] with `extra_config` appended to the project
/// config before the first install.
fn project_ready_to_update_with(extra_config: &str) -> (TempDir, TempDir, TempDir, TempDir) {
    let (work, bare) = make_harbor();
    let project = TempDir::new().unwrap();
    let config_home = TempDir::new().unwrap();
    let p = project.path().to_str().unwrap().to_string();

    std::fs::create_dir_all(project.path().join(".quay")).unwrap();
    std::fs::write(
        project.path().join(".quay/config.toml"),
        project_config_for_url(bare.path().to_str().unwrap()) + extra_config,
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
        .stderr(predicates::str::contains("kept 1 file "))
        .stderr(predicates::str::contains("notes.md"))
        .stderr(predicates::str::contains("--delete-extra"));

    assert!(project.path().join(".agents/skills/foo/notes.md").exists());
    assert_v2_landed(&project);
}

#[test]
fn update_delete_extra_removes_them() {
    let (project, _work, _bare, config_home) = project_ready_to_update();
    let p = project.path().to_str().unwrap().to_string();

    quay()
        .env("XDG_CONFIG_HOME", config_home.path())
        .args(["--project", &p, "update", "foo", "--delete-extra"])
        .assert()
        .success()
        // Unattended, this note is the only record that anything was removed.
        .stderr(predicates::str::contains("deleting 1 file "))
        .stderr(predicates::str::contains("notes.md"))
        .stderr(predicates::str::contains("--keep-extra"));

    assert!(
        !project.path().join(".agents/skills/foo/notes.md").exists(),
        "--delete-extra must remove the extra file"
    );
    let installed =
        std::fs::read_to_string(project.path().join(".agents/skills/foo/SKILL.md")).unwrap();
    assert!(
        installed.contains("body 2.0.0"),
        "the update must actually have landed v2: {installed}"
    );
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
        .stderr(predicates::str::contains("--delete-extra").not());

    assert!(project.path().join(".agents/skills/foo/notes.md").exists());
    assert_v2_landed(&project);
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
    assert_v2_landed(&project);
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
    assert_v2_landed(&project);
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
    assert_v2_landed(&project);
}

/// `--delete-extra` under `--json` is the unattended case with the least
/// record: stderr is often discarded, so stdout must say what was removed.
#[test]
fn update_json_records_the_files_it_deleted() {
    let (project, _work, _bare, config_home) = project_ready_to_update();
    let p = project.path().to_str().unwrap().to_string();

    let out = quay()
        .env("XDG_CONFIG_HOME", config_home.path())
        .args(["--project", &p, "update", "foo", "--delete-extra", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let v: serde_json::Value = serde_json::from_slice(&out).expect("valid JSON");
    assert_eq!(v[0]["name"], "foo", "{v}");
    assert_eq!(
        v[0]["deleted_extras"],
        serde_json::json!(["notes.md"]),
        "{v}"
    );
}

#[test]
fn update_json_keep_records_no_deletions() {
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

    let v: serde_json::Value = serde_json::from_slice(&out).expect("valid JSON");
    assert_eq!(v[0]["deleted_extras"], serde_json::json!([]), "{v}");
}

/// After `--delete-extra`, a copy mirror that still held the extra kept it, so
/// `quay diff` went on reporting a file the release notes said was gone.
#[test]
fn update_carries_the_new_version_and_the_deletion_into_a_copy_mirror() {
    let (project, _work, _bare, config_home) = project_ready_to_update_with(COPY_MIRROR);
    let p = project.path().to_str().unwrap().to_string();
    let mirror = project.path().join(".cursor/rules/foo");
    // Sync notes.md into the mirror first, so it holds the extra too.
    quay()
        .env("XDG_CONFIG_HOME", config_home.path())
        .args(["--project", &p, "link"])
        .assert()
        .success();
    assert!(
        mirror.join("notes.md").exists(),
        "precondition: mirror holds the extra"
    );

    quay()
        .env("XDG_CONFIG_HOME", config_home.path())
        .args(["--project", &p, "update", "foo", "--delete-extra"])
        .assert()
        .success()
        .stderr(predicates::str::contains("warning").not());

    let mirrored = std::fs::read_to_string(mirror.join("SKILL.md")).unwrap();
    assert!(
        mirrored.contains("body 2.0.0"),
        "mirror must follow the update: {mirrored}"
    );
    assert!(
        !mirror.join("notes.md").exists(),
        "the deleted extra must not survive in the mirror"
    );
}

/// Publish `version` with exactly `files` (beside SKILL.md) under skills/foo,
/// deleting any other file there — what a hub-side `git rm` looks like.
fn publish_with(work: &TempDir, version: &str, files: &[(&str, &str)]) {
    let dir = work.path().join("skills/foo");
    std::fs::remove_dir_all(&dir).unwrap();
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("SKILL.md"), skill_body(version)).unwrap();
    let mut names = vec!["SKILL.md"];
    for (name, body) in files {
        std::fs::write(dir.join(name), body).unwrap();
        names.push(name);
    }
    std::fs::write(
        work.path().join("registry.json"),
        registry_json_with(version, &names),
    )
    .unwrap();
    git(work.path(), &["add", "-A"]);
    git(work.path(), &["commit", "-m", version]);
    git(work.path(), &["push", "origin", "main:main"]);
}

/// The case the feature exists for: the hub deleted `legacy.md`, and before
/// this an update resurrected it forever. Installed from the hub, not
/// hand-written — so it also proves a file quay itself put there is offered.
#[test]
fn a_file_the_hub_deleted_is_offered_and_removed_with_delete_extra() {
    let (work, bare) = make_harbor();
    publish_with(&work, "1.5.0", &[("legacy.md", "old reference")]);
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
    let legacy = project.path().join(".agents/skills/foo/legacy.md");
    assert!(legacy.exists(), "precondition: v1.5 installed legacy.md");

    publish_with(&work, "2.0.0", &[]);
    quay()
        .env("XDG_CONFIG_HOME", config_home.path())
        .args(["--project", &p, "update", "foo", "--delete-extra"])
        .assert()
        .success()
        .stderr(predicates::str::contains("legacy.md"));

    assert!(
        !legacy.exists(),
        "the hub's deletion must reach the install"
    );
    assert_v2_landed(&project);
}
