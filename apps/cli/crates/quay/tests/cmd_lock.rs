use assert_cmd::Command;
use assert_fs::prelude::*;

/// `quay lock` with skills on disk writes skills-lock.json listing them.
#[test]
fn lock_generates_lockfile_from_scan() {
    let project = assert_fs::TempDir::new().unwrap();
    project
        .child(".agents/skills/csv-parse/SKILL.md")
        .write_str("---\nname: csv-parse\ndescription: parse csv\nversion: 1.0.0\n---\nbody\n")
        .unwrap();

    Command::cargo_bin("quay").unwrap()
        .args(["lock"])
        .current_dir(project.path())
        .assert()
        .success();

    let lock = std::fs::read_to_string(project.path().join("skills-lock.json")).unwrap();
    assert!(lock.contains("\"csv-parse\""));
    assert!(lock.contains("\"computedHash\""));
    assert!(lock.contains("\"skillPath\""));
    assert!(lock.contains("\"version\": 1"));
}

/// A clean lockfile that matches disk → --check succeeds.
#[test]
fn check_passes_when_lock_matches_disk() {
    let project = assert_fs::TempDir::new().unwrap();
    project.child(".agents/skills/csv-parse/SKILL.md")
        .write_str("---\nname: csv-parse\ndescription: d\nversion: 1.0.0\n---\nbody\n").unwrap();
    Command::cargo_bin("quay").unwrap().args(["lock"]).current_dir(project.path()).assert().success();
    Command::cargo_bin("quay").unwrap().args(["lock", "--check"]).current_dir(project.path()).assert().success();
}

/// An untracked skill (on disk, not in lock) fails --check (strict policy).
#[test]
fn check_fails_on_untracked_skill() {
    let project = assert_fs::TempDir::new().unwrap();
    project.child(".agents/skills/csv-parse/SKILL.md")
        .write_str("---\nname: csv-parse\ndescription: d\nversion: 1.0.0\n---\nbody\n").unwrap();
    Command::cargo_bin("quay").unwrap().args(["lock"]).current_dir(project.path()).assert().success();
    project.child(".agents/skills/new-one/SKILL.md")
        .write_str("---\nname: new-one\ndescription: d\nversion: 1.0.0\n---\nbody\n").unwrap();
    Command::cargo_bin("quay").unwrap().args(["lock", "--check"]).current_dir(project.path())
        .assert().failure().stderr(predicates::str::contains("untracked"));
}

/// A modified skill (hash differs) fails --check.
#[test]
fn check_fails_on_modified_skill() {
    let project = assert_fs::TempDir::new().unwrap();
    let f = project.child(".agents/skills/csv-parse/SKILL.md");
    f.write_str("---\nname: csv-parse\ndescription: d\nversion: 1.0.0\n---\nbody\n").unwrap();
    Command::cargo_bin("quay").unwrap().args(["lock"]).current_dir(project.path()).assert().success();
    f.write_str("---\nname: csv-parse\ndescription: d\nversion: 1.0.0\n---\nCHANGED\n").unwrap();
    Command::cargo_bin("quay").unwrap().args(["lock", "--check"]).current_dir(project.path())
        .assert().failure().stderr(predicates::str::contains("modified"));
}

/// A lock entry whose file is gone fails --check.
#[test]
fn check_fails_on_missing_skill() {
    let project = assert_fs::TempDir::new().unwrap();
    project.child(".agents/skills/csv-parse/SKILL.md")
        .write_str("---\nname: csv-parse\ndescription: d\nversion: 1.0.0\n---\nbody\n").unwrap();
    Command::cargo_bin("quay").unwrap().args(["lock"]).current_dir(project.path()).assert().success();
    std::fs::remove_dir_all(project.path().join(".agents/skills/csv-parse")).unwrap();
    Command::cargo_bin("quay").unwrap().args(["lock", "--check"]).current_dir(project.path())
        .assert().failure().stderr(predicates::str::contains("missing"));
}

/// --sync with everything already present is a successful no-op.
#[test]
fn sync_noop_when_all_present() {
    let project = assert_fs::TempDir::new().unwrap();
    project.child(".agents/skills/csv-parse/SKILL.md")
        .write_str("---\nname: csv-parse\ndescription: d\nversion: 1.0.0\n---\nbody\n").unwrap();
    Command::cargo_bin("quay").unwrap().args(["lock"]).current_dir(project.path()).assert().success();

    Command::cargo_bin("quay").unwrap().args(["lock", "--sync"]).current_dir(project.path())
        .assert().success().stdout(predicates::str::contains("up to date"));
}

/// --sync notes that it skips a non-installable (local) source type.
#[test]
fn sync_skips_local_entries_with_note() {
    let project = assert_fs::TempDir::new().unwrap();
    std::fs::write(project.path().join("skills-lock.json"),
        "{\n  \"version\": 1,\n  \"skills\": {\n    \"hand-made\": {\n      \"source\": \".agents/skills/hand-made/SKILL.md\",\n      \"sourceType\": \"local\",\n      \"skillPath\": \".agents/skills/hand-made/SKILL.md\",\n      \"computedHash\": \"0000000000000000000000000000000000000000000000000000000000000000\"\n    }\n  }\n}").unwrap();

    Command::cargo_bin("quay").unwrap().args(["lock", "--sync"]).current_dir(project.path())
        .assert().success().stdout(predicates::str::contains("skip"));
}

/// --heal makes a drifted repo pass --check, and is idempotent.
#[test]
fn heal_reconciles_and_is_idempotent() {
    let project = assert_fs::TempDir::new().unwrap();
    project.child(".agents/skills/csv-parse/SKILL.md")
        .write_str("---\nname: csv-parse\ndescription: d\nversion: 1.0.0\n---\nbody\n").unwrap();
    Command::cargo_bin("quay").unwrap().args(["lock"]).current_dir(project.path()).assert().success();

    // Introduce drift: a new untracked skill.
    project.child(".agents/skills/new-one/SKILL.md")
        .write_str("---\nname: new-one\ndescription: d\nversion: 1.0.0\n---\nbody\n").unwrap();

    // Heal, then --check passes.
    Command::cargo_bin("quay").unwrap().args(["lock", "--heal"]).current_dir(project.path()).assert().success();
    Command::cargo_bin("quay").unwrap().args(["lock", "--check"]).current_dir(project.path()).assert().success();

    // Idempotence: capture file, heal again, assert unchanged.
    let path = project.path().join("skills-lock.json");
    let before = std::fs::read_to_string(&path).unwrap();
    Command::cargo_bin("quay").unwrap().args(["lock", "--heal"]).current_dir(project.path()).assert().success();
    let after = std::fs::read_to_string(&path).unwrap();
    assert_eq!(before, after, "second --heal must not change the lockfile");
}
