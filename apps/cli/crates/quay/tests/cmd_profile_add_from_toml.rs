//! Integration tests for `quay profile add <name> --from-toml <path|->`.

use assert_cmd::Command;
use assert_fs::prelude::*;

fn empty_config(dir: &assert_fs::TempDir) -> std::path::PathBuf {
    let p = dir.child("user.toml");
    std::fs::write(p.path(), "").unwrap();
    p.path().to_path_buf()
}

#[test]
fn add_from_toml_via_stdin_creates_profile_with_remotes() {
    let tmp = assert_fs::TempDir::new().unwrap();
    let cfg = empty_config(&tmp);

    let toml = r#"
        email = "demo@example.com"
        [remotes.azure]
        url = "git@ssh.dev.azure.com:v3/org/proj/repo"
        provider = "azuredevops"
        push_mode = "direct"
        default = true
    "#;

    Command::cargo_bin("quay")
        .unwrap()
        .args([
            "profile",
            "add",
            "demo",
            "--from-toml",
            "-",
            "--user-config",
            cfg.to_str().unwrap(),
            "--activate",
        ])
        .write_stdin(toml)
        .assert()
        .success();

    let saved = std::fs::read_to_string(&cfg).unwrap();
    assert!(saved.contains("profiles.demo"), "missing profile: {saved}");
    assert!(
        saved.contains("active_profile = \"demo\""),
        "missing active: {saved}"
    );
    assert!(saved.contains("demo@example.com"), "missing email: {saved}");
    assert!(
        saved.contains("[profiles.demo.remotes.azure]"),
        "missing remote: {saved}"
    );
}

#[test]
fn add_from_toml_via_file_creates_profile() {
    let tmp = assert_fs::TempDir::new().unwrap();
    let cfg = empty_config(&tmp);

    // Write a profile TOML file.
    let profile_file = tmp.child("ci-profile.toml");
    profile_file
        .write_str(
            r#"
email = "ci@example.com"
[remotes.github]
url = "git@github.com:org/skills.git"
push_mode = "pr"
default = true
"#,
        )
        .unwrap();

    Command::cargo_bin("quay")
        .unwrap()
        .args([
            "profile",
            "add",
            "ci",
            "--from-toml",
            profile_file.path().to_str().unwrap(),
            "--user-config",
            cfg.to_str().unwrap(),
        ])
        .assert()
        .success();

    let saved = std::fs::read_to_string(&cfg).unwrap();
    assert!(saved.contains("profiles.ci"), "missing ci profile: {saved}");
    assert!(saved.contains("ci@example.com"), "missing email: {saved}");
}

#[test]
fn add_from_toml_auto_detects_provider_when_absent() {
    let tmp = assert_fs::TempDir::new().unwrap();
    let cfg = empty_config(&tmp);

    let toml = r#"
        email = "user@example.com"
        [remotes.gh]
        url = "git@github.com:org/skills.git"
        default = true
    "#;

    Command::cargo_bin("quay")
        .unwrap()
        .args([
            "profile",
            "add",
            "work",
            "--from-toml",
            "-",
            "--user-config",
            cfg.to_str().unwrap(),
        ])
        .write_stdin(toml)
        .assert()
        .success();

    let saved = std::fs::read_to_string(&cfg).unwrap();
    // Provider should have been auto-detected as github.
    assert!(
        saved.contains("provider = \"github\""),
        "missing provider: {saved}"
    );
}

#[test]
fn add_from_toml_and_interactive_are_mutually_exclusive() {
    let tmp = assert_fs::TempDir::new().unwrap();
    let cfg = empty_config(&tmp);

    Command::cargo_bin("quay")
        .unwrap()
        .args([
            "profile",
            "add",
            "-i",
            "--from-toml",
            "-",
            "--user-config",
            cfg.to_str().unwrap(),
        ])
        .write_stdin("")
        .assert()
        .failure();
}

#[test]
fn add_from_toml_and_email_are_mutually_exclusive() {
    let tmp = assert_fs::TempDir::new().unwrap();
    let cfg = empty_config(&tmp);

    Command::cargo_bin("quay")
        .unwrap()
        .args([
            "profile",
            "add",
            "demo",
            "--from-toml",
            "-",
            "--email",
            "x@y",
            "--user-config",
            cfg.to_str().unwrap(),
        ])
        .write_stdin("")
        .assert()
        .failure();
}

#[test]
fn add_from_toml_rejects_duplicate_profile() {
    let tmp = assert_fs::TempDir::new().unwrap();
    let cfg = tmp.child("user.toml");
    cfg.write_str(
        r#"
active_profile = "demo"
[profiles.demo.user]
email = "demo@example.com"
"#,
    )
    .unwrap();

    let toml = r#"email = "other@example.com""#;

    Command::cargo_bin("quay")
        .unwrap()
        .args([
            "profile",
            "add",
            "demo",
            "--from-toml",
            "-",
            "--user-config",
            cfg.path().to_str().unwrap(),
        ])
        .write_stdin(toml)
        .assert()
        .failure()
        .stderr(predicates::str::contains("already exists"));
}
