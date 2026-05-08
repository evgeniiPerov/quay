use assert_cmd::Command;
use assert_fs::prelude::*;

#[test]
fn validate_passes_for_well_formed_skill() {
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
    // Fill in description so validate passes (template description is empty).
    let md = dir.child(".agents/skills/csv-parse/SKILL.md");
    let mut body = std::fs::read_to_string(md.path()).unwrap();
    body = body.replace("description: \n", "description: Parse CSV\n");
    std::fs::write(md.path(), body).unwrap();
    Command::cargo_bin("quay")
        .unwrap()
        .args(["--project", p, "validate", "csv-parse"])
        .assert()
        .success()
        .stdout(predicates::str::contains("ok: csv-parse v0.1.0"));
}

#[test]
fn validate_fails_for_missing_skill() {
    let dir = assert_fs::TempDir::new().unwrap();
    let p = dir.path().to_str().unwrap();
    Command::cargo_bin("quay")
        .unwrap()
        .args(["--project", p, "init"])
        .assert()
        .success();
    Command::cargo_bin("quay")
        .unwrap()
        .args(["--project", p, "validate", "nonexistent"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("skill not found"));
}

#[test]
fn validate_fails_for_bad_frontmatter() {
    let dir = assert_fs::TempDir::new().unwrap();
    let p = dir.path().to_str().unwrap();
    Command::cargo_bin("quay")
        .unwrap()
        .args(["--project", p, "init"])
        .assert()
        .success();
    let skill_dir = dir.child(".agents/skills/broken");
    std::fs::create_dir_all(skill_dir.path()).unwrap();
    std::fs::write(
        skill_dir.path().join("SKILL.md"),
        "---\nname: broken\ndescription: x\nversion: not-semver\n---\nbody\n",
    )
    .unwrap();
    Command::cargo_bin("quay")
        .unwrap()
        .args(["--project", p, "validate", "broken"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("semver"));
}
