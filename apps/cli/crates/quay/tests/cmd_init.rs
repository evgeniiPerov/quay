use assert_cmd::Command;
use assert_fs::prelude::*;

#[test]
fn init_creates_quay_dir_and_config_file() {
    let dir = assert_fs::TempDir::new().unwrap();
    Command::cargo_bin("quay")
        .unwrap()
        .args(["--project", dir.path().to_str().unwrap(), "init"])
        .assert()
        .success();
    dir.child(".quay/config.toml")
        .assert(predicates::path::exists());
    dir.child(".agents/skills")
        .assert(predicates::path::is_dir());
}

#[test]
fn init_is_idempotent() {
    let dir = assert_fs::TempDir::new().unwrap();
    for _ in 0..2 {
        Command::cargo_bin("quay")
            .unwrap()
            .args(["--project", dir.path().to_str().unwrap(), "init"])
            .assert()
            .success();
    }
    dir.child(".quay/config.toml")
        .assert(predicates::path::exists());
}

#[test]
fn init_json_output() {
    let dir = assert_fs::TempDir::new().unwrap();
    let output = Command::cargo_bin("quay")
        .unwrap()
        .args(["--project", dir.path().to_str().unwrap(), "--json", "init"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("output is valid JSON");
    assert!(parsed.get("config_path").is_some());
    assert!(parsed.get("skills_dir").is_some());
    assert_eq!(
        parsed.get("created_config"),
        Some(&serde_json::Value::Bool(true))
    );
}
