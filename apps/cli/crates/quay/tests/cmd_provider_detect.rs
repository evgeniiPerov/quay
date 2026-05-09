//! Integration tests for Plan 7a provider-field persistence.
//! Verifies that `--provider` is persisted in the project config TOML and that
//! omitting it leaves the field out entirely (backwards-compatible with pre-7a
//! config files).

use assert_cmd::Command;
use assert_fs::prelude::*;

fn quay() -> Command {
    Command::cargo_bin("quay").unwrap()
}

#[test]
fn add_with_explicit_provider_persists_field() {
    let tmp = assert_fs::TempDir::new().unwrap();
    let p = tmp.path().to_str().unwrap();

    quay().args(["--project", p, "init"]).assert().success();

    quay()
        .args([
            "--project",
            p,
            "remote",
            "add",
            "skills-hub",
            "https://gitlab.example.com/o/r.git",
            "--provider",
            "gitlab",
        ])
        .assert()
        .success();

    let cfg_path = tmp.child(".quay/config.toml");
    let cfg = std::fs::read_to_string(cfg_path.path()).unwrap();
    assert!(
        cfg.contains("provider = \"gitlab\""),
        "expected provider field in project config, got:\n{}",
        cfg
    );
}

#[test]
fn add_without_provider_field_omits_in_toml() {
    let tmp = assert_fs::TempDir::new().unwrap();
    let p = tmp.path().to_str().unwrap();

    quay().args(["--project", p, "init"]).assert().success();

    quay()
        .args([
            "--project",
            p,
            "remote",
            "add",
            "h",
            "git@github.com:o/r.git",
        ])
        .assert()
        .success();

    let cfg_path = tmp.child(".quay/config.toml");
    let cfg = std::fs::read_to_string(cfg_path.path()).unwrap();
    assert!(
        !cfg.contains("provider ="),
        "expected no provider field in project config, got:\n{}",
        cfg
    );
}
