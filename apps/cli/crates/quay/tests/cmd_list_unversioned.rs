//! `quay list` shows "unversioned" (not 0.0.0) for hand-written skills.

use assert_cmd::Command;
use assert_fs::prelude::*;
use predicates::prelude::PredicateBooleanExt;

#[test]
fn list_shows_unversioned_for_freestyle_skill() {
    let project = assert_fs::TempDir::new().unwrap();
    project
        .child(".agents/skills/hello/SKILL.md")
        .write_str("# /hello\nJust do the thing.\n")
        .unwrap();

    Command::cargo_bin("quay")
        .unwrap()
        .args(["--project", project.path().to_str().unwrap(), "list"])
        .assert()
        .success()
        .stdout(predicates::str::contains("unversioned"))
        .stdout(predicates::str::contains("0.0.0").not());
}
