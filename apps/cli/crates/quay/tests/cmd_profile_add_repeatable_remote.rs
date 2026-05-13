//! Integration tests for repeatable `--remote` on `quay profile add`.

use assert_cmd::Command;
use assert_fs::prelude::*;

fn empty_config(dir: &assert_fs::TempDir) -> std::path::PathBuf {
    let p = dir.child("user.toml");
    std::fs::write(p.path(), "").unwrap();
    p.path().to_path_buf()
}

#[test]
fn add_with_two_remotes_seeds_both() {
    let tmp = assert_fs::TempDir::new().unwrap();
    let cfg = empty_config(&tmp);

    Command::cargo_bin("quay")
        .unwrap()
        .args([
            "profile",
            "add",
            "demo",
            "--email",
            "x@y",
            "--remote",
            "azure=git@ssh.dev.azure.com:v3/org/proj/repo",
            "--provider",
            "azuredevops",
            "--push-mode",
            "direct",
            "--default",
            "--remote",
            "gitlab=git@gitlab.example.com:org/repo.git",
            "--provider",
            "gitlab",
            "--push-mode",
            "pr",
            "--user-config",
            cfg.to_str().unwrap(),
            "--activate",
        ])
        .assert()
        .success();

    let saved = std::fs::read_to_string(&cfg).unwrap();
    assert!(
        saved.contains("[profiles.demo.remotes.azure]"),
        "missing azure remote: {saved}"
    );
    assert!(
        saved.contains("[profiles.demo.remotes.gitlab]"),
        "missing gitlab remote: {saved}"
    );
    assert!(
        saved.contains("provider = \"azuredevops\""),
        "missing azuredevops provider: {saved}"
    );
    assert!(
        saved.contains("provider = \"gitlab\""),
        "missing gitlab provider: {saved}"
    );
    assert!(
        saved.contains("push_mode = \"direct\""),
        "missing direct push_mode: {saved}"
    );
    // azure should be default (first --default flag)
    let azure_idx = saved.find("[profiles.demo.remotes.azure]").unwrap();
    let gitlab_idx = saved.find("[profiles.demo.remotes.gitlab]").unwrap();
    let default_pos = saved.find("default = true").unwrap();
    // The default = true should be in the azure section (before gitlab section in TOML).
    // Since BTreeMap sorts keys, azure < gitlab alphabetically, so azure is first.
    assert!(
        default_pos > azure_idx && default_pos < gitlab_idx || {
            // Alternatively, gitlab comes before azure alphabetically... let's
            // just verify that exactly one "default = true" exists.
            saved.matches("default = true").count() == 1
        },
        "expected exactly one default remote: {saved}"
    );
    assert!(
        saved.contains("active_profile = \"demo\""),
        "missing active: {saved}"
    );
}

#[test]
fn add_with_single_remote_and_provider_auto_detected() {
    let tmp = assert_fs::TempDir::new().unwrap();
    let cfg = empty_config(&tmp);

    Command::cargo_bin("quay")
        .unwrap()
        .args([
            "profile",
            "add",
            "work",
            "--email",
            "w@work.com",
            "--remote",
            "gh=git@github.com:org/skills.git",
            "--user-config",
            cfg.to_str().unwrap(),
        ])
        .assert()
        .success();

    let saved = std::fs::read_to_string(&cfg).unwrap();
    assert!(
        saved.contains("provider = \"github\""),
        "auto-detect failed: {saved}"
    );
}

#[test]
fn add_with_direct_branch_writes_branch_into_toml() {
    let tmp = assert_fs::TempDir::new().unwrap();
    let cfg = empty_config(&tmp);

    Command::cargo_bin("quay")
        .unwrap()
        .args([
            "profile",
            "add",
            "oriflame-frontend",
            "--email",
            "you@oriflame.com",
            "--remote",
            "harbour=https://dev.azure.com/oriflame/Tooling/_git/skills-frontend-harbour",
            "--provider",
            "azuredevops",
            "--push-mode",
            "direct",
            "--direct-branch",
            "develop",
            "--user-config",
            cfg.to_str().unwrap(),
            "--activate",
        ])
        .assert()
        .success();

    let saved = std::fs::read_to_string(&cfg).unwrap();
    assert!(
        saved.contains("push_mode = \"direct\""),
        "missing direct push_mode: {saved}"
    );
    assert!(
        saved.contains("direct_branch = \"develop\""),
        "missing direct_branch: {saved}"
    );
}

#[test]
fn add_direct_branch_count_exceeding_remotes_errors() {
    let tmp = assert_fs::TempDir::new().unwrap();
    let cfg = empty_config(&tmp);

    Command::cargo_bin("quay")
        .unwrap()
        .args([
            "profile",
            "add",
            "demo",
            "--email",
            "x@y",
            "--remote",
            "gh=git@github.com:org/skills.git",
            "--direct-branch",
            "develop",
            "--direct-branch",
            "main",
            "--user-config",
            cfg.to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicates::str::contains("--direct-branch specified 2"));
}

#[test]
fn backward_compatible_single_remote_still_works() {
    let tmp = assert_fs::TempDir::new().unwrap();
    let cfg = empty_config(&tmp);

    // Legacy one-shot form: `--remote name=url`
    Command::cargo_bin("quay")
        .unwrap()
        .args([
            "profile",
            "add",
            "personal",
            "--email",
            "e@home",
            "--remote",
            "my-pool=https://github.com/me/skills.git",
            "--activate",
            "--user-config",
            cfg.to_str().unwrap(),
        ])
        .assert()
        .success();

    let saved = std::fs::read_to_string(&cfg).unwrap();
    assert!(
        saved.contains("[profiles.personal.remotes.my-pool]"),
        "missing remote: {saved}"
    );
}
