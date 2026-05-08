use assert_cmd::Command;
use assert_fs::prelude::*;

#[test]
fn create_scaffolds_skill_md() {
    let dir = assert_fs::TempDir::new().unwrap();
    let p = dir.path().to_str().unwrap();
    Command::cargo_bin("quay")
        .unwrap()
        .args(["--project", p, "init"])
        .assert()
        .success();
    Command::cargo_bin("quay")
        .unwrap()
        .args(["--project", p, "create", "csv-parse"])
        .assert()
        .success()
        .stdout(predicates::str::contains("created"));
    let body =
        std::fs::read_to_string(dir.child(".agents/skills/csv-parse/SKILL.md").path()).unwrap();
    assert!(body.contains("name: csv-parse"));
    assert!(body.contains("version: 0.1.0"));
}

#[test]
fn create_rejects_non_kebab_name() {
    let dir = assert_fs::TempDir::new().unwrap();
    let p = dir.path().to_str().unwrap();
    Command::cargo_bin("quay")
        .unwrap()
        .args(["--project", p, "init"])
        .assert()
        .success();
    Command::cargo_bin("quay")
        .unwrap()
        .args(["--project", p, "create", "Bad_Name"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("kebab-case"));
}

#[test]
fn create_refuses_to_overwrite() {
    let dir = assert_fs::TempDir::new().unwrap();
    let p = dir.path().to_str().unwrap();
    Command::cargo_bin("quay")
        .unwrap()
        .args(["--project", p, "init"])
        .assert()
        .success();
    Command::cargo_bin("quay")
        .unwrap()
        .args(["--project", p, "create", "csv-parse"])
        .assert()
        .success();
    Command::cargo_bin("quay")
        .unwrap()
        .args(["--project", p, "create", "csv-parse"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("already exists"));
}
