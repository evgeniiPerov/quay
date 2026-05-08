use assert_cmd::Command;
use assert_fs::prelude::*;

fn make_project_with_skill(dir: &assert_fs::TempDir, skill: &str) {
    let p = dir.path().to_str().unwrap();
    Command::cargo_bin("quay")
        .unwrap()
        .args(["--project", p, "init"])
        .assert()
        .success();
    let skill_dir = dir.child(format!(".agents/skills/{}", skill));
    std::fs::create_dir_all(skill_dir.path()).unwrap();
    std::fs::write(
        skill_dir.path().join("SKILL.md"),
        b"---\nname: x\ndescription: y\nversion: 0.1.0\n---\nbody\n",
    )
    .unwrap();
}

/// Adds a mirror entry to the project config by reading and rewriting it via
/// `ProjectConfigFile` so that duplicate TOML table keys are never produced.
fn append_mirror_to_config(dir: &assert_fs::TempDir, mirror_path: &str, strategy: &str) {
    let cfg_path = dir.child(".quay/config.toml").path().to_path_buf();
    let text = std::fs::read_to_string(&cfg_path).unwrap();
    let mut doc: toml::Value = toml::from_str(&text).unwrap();

    let mirrors = doc
        .get_mut("install")
        .and_then(|i| i.get_mut("mirrors"))
        .expect("init must have written [install] with mirrors");

    let entry = toml::Value::Table({
        let mut t = toml::map::Map::new();
        t.insert("path".into(), toml::Value::String(mirror_path.into()));
        t.insert("strategy".into(), toml::Value::String(strategy.into()));
        t
    });

    if let toml::Value::Array(arr) = mirrors {
        arr.push(entry);
    }

    std::fs::write(&cfg_path, toml::to_string_pretty(&doc).unwrap()).unwrap();
}

#[test]
fn link_creates_mirrors_for_installed_skills() {
    let dir = assert_fs::TempDir::new().unwrap();
    make_project_with_skill(&dir, "csv-parse");
    append_mirror_to_config(&dir, ".claude/skills", "symlink");

    let p = dir.path().to_str().unwrap();
    Command::cargo_bin("quay")
        .unwrap()
        .args(["--project", p, "link"])
        .assert()
        .success()
        .stdout(predicates::str::contains("created  csv-parse"));

    let mirror = dir.path().join(".claude/skills/csv-parse");
    assert!(std::fs::symlink_metadata(&mirror)
        .unwrap()
        .file_type()
        .is_symlink());
}

#[test]
fn link_check_succeeds_when_clean() {
    let dir = assert_fs::TempDir::new().unwrap();
    make_project_with_skill(&dir, "csv-parse");
    append_mirror_to_config(&dir, ".claude/skills", "symlink");

    let p = dir.path().to_str().unwrap();
    Command::cargo_bin("quay")
        .unwrap()
        .args(["--project", p, "link"])
        .assert()
        .success();
    Command::cargo_bin("quay")
        .unwrap()
        .args(["--project", p, "link", "check"])
        .assert()
        .success()
        .stdout(predicates::str::contains("ok: all mirrors intact"));
}

#[test]
fn link_check_fails_when_mirror_missing() {
    let dir = assert_fs::TempDir::new().unwrap();
    make_project_with_skill(&dir, "csv-parse");
    append_mirror_to_config(&dir, ".claude/skills", "symlink");

    let p = dir.path().to_str().unwrap();
    Command::cargo_bin("quay")
        .unwrap()
        .args(["--project", p, "link", "check"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("drift: csv-parse"));
}

#[test]
fn link_add_writes_config_and_applies() {
    let dir = assert_fs::TempDir::new().unwrap();
    make_project_with_skill(&dir, "csv-parse");

    let p = dir.path().to_str().unwrap();
    Command::cargo_bin("quay")
        .unwrap()
        .args([
            "--project",
            p,
            "link",
            "add",
            ".claude/skills",
            "--strategy",
            "symlink",
        ])
        .assert()
        .success();

    let written = std::fs::read_to_string(dir.child(".quay/config.toml").path()).unwrap();
    assert!(written.contains(".claude/skills"));
    let mirror = dir.path().join(".claude/skills/csv-parse");
    assert!(std::fs::symlink_metadata(&mirror)
        .unwrap()
        .file_type()
        .is_symlink());
}

#[test]
fn link_remove_deletes_config_entry_only() {
    let dir = assert_fs::TempDir::new().unwrap();
    make_project_with_skill(&dir, "csv-parse");
    append_mirror_to_config(&dir, ".claude/skills", "symlink");

    let p = dir.path().to_str().unwrap();
    Command::cargo_bin("quay")
        .unwrap()
        .args(["--project", p, "link"])
        .assert()
        .success();
    let mirror = dir.path().join(".claude/skills/csv-parse");
    assert!(mirror.exists());

    Command::cargo_bin("quay")
        .unwrap()
        .args(["--project", p, "link", "remove", ".claude/skills"])
        .assert()
        .success();

    // Mirror dir is intentionally not deleted.
    assert!(mirror.exists());
    let written = std::fs::read_to_string(dir.child(".quay/config.toml").path()).unwrap();
    assert!(!written.contains(".claude/skills"));
}

#[test]
fn link_force_replaces_conflicting_dir() {
    let dir = assert_fs::TempDir::new().unwrap();
    make_project_with_skill(&dir, "csv-parse");
    append_mirror_to_config(&dir, ".claude/skills", "symlink");

    let conflict = dir.path().join(".claude/skills/csv-parse");
    std::fs::create_dir_all(&conflict).unwrap();
    std::fs::write(conflict.join("user-file.md"), b"theirs").unwrap();

    let p = dir.path().to_str().unwrap();
    Command::cargo_bin("quay")
        .unwrap()
        .args(["--project", p, "link"])
        .assert()
        .failure();
    Command::cargo_bin("quay")
        .unwrap()
        .args(["--project", p, "link", "--force"])
        .assert()
        .success();
    let mirror = dir.path().join(".claude/skills/csv-parse");
    assert!(std::fs::symlink_metadata(&mirror)
        .unwrap()
        .file_type()
        .is_symlink());
}
