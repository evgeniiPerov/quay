use assert_cmd::Command;
use assert_fs::prelude::*;

#[test]
fn scan_lists_three_format_variants() {
    let project = assert_fs::TempDir::new().unwrap();
    project
        .child(".agents/skills/a-front/SKILL.md")
        .write_str("---\nname: a-front\ndescription: front\n---\nbody\n")
        .unwrap();
    project
        .child(".agents/skills/b-slash/SKILL.md")
        .write_str("# /b-slash\n\nA slash skill.\n")
        .unwrap();
    project
        .child(".agents/skills/c-free/SKILL.md")
        .write_str("Just markdown.\n")
        .unwrap();

    Command::cargo_bin("quay")
        .unwrap()
        .args(["scan", "--json"])
        .current_dir(project.path())
        .assert()
        .success()
        .stdout(predicates::str::contains("\"a-front\""))
        .stdout(predicates::str::contains("\"b-slash\""))
        .stdout(predicates::str::contains("\"c-free\""))
        .stdout(predicates::str::contains("\"frontmatter\""))
        .stdout(predicates::str::contains("\"slash_command\""))
        .stdout(predicates::str::contains("\"freestyle\""));
}

#[test]
fn scan_table_shows_local_status_when_no_lockfile() {
    let project = assert_fs::TempDir::new().unwrap();
    project
        .child(".agents/skills/foo/SKILL.md")
        .write_str("---\nname: foo\ndescription: f\n---\n")
        .unwrap();

    Command::cargo_bin("quay")
        .unwrap()
        .arg("scan")
        .current_dir(project.path())
        .assert()
        .success()
        .stdout(predicates::str::contains("foo"))
        .stdout(predicates::str::contains("local"));
}

#[test]
fn scan_empty_dir_prints_helpful_message() {
    let project = assert_fs::TempDir::new().unwrap();
    project.child(".agents/skills").create_dir_all().unwrap();

    Command::cargo_bin("quay")
        .unwrap()
        .arg("scan")
        .current_dir(project.path())
        .assert()
        .success()
        .stdout(predicates::str::contains("no local skills found"));
}
