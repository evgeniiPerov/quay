//! Integration tests for `quay remote edit`.

use assert_cmd::Command;
use assert_fs::TempDir;
use predicates::prelude::*;

// ── helpers ───────────────────────────────────────────────────────────────────

fn quay() -> Command {
    Command::cargo_bin("quay").unwrap()
}

/// Initialise a project dir and add two remotes (`first` + `second`) with
/// `first` marked as the default. Returns the project directory path string.
fn setup_project_with_two_remotes(dir: &TempDir) -> String {
    let p = dir.path().to_str().unwrap().to_string();

    quay().args(["--project", &p, "init"]).assert().success();

    quay()
        .args([
            "--project",
            &p,
            "remote",
            "add",
            "first",
            "https://github.com/org/skills-first.git",
            "--default",
        ])
        .assert()
        .success();

    quay()
        .args([
            "--project",
            &p,
            "remote",
            "add",
            "second",
            "https://github.com/org/skills-second.git",
        ])
        .assert()
        .success();

    p
}

// ── patch URL only ────────────────────────────────────────────────────────────

#[test]
fn edit_url_updates_remote() {
    let dir = TempDir::new().unwrap();
    let p = setup_project_with_two_remotes(&dir);

    quay()
        .args([
            "--project",
            &p,
            "remote",
            "edit",
            "first",
            "--url",
            "https://github.com/org/skills-v2.git",
        ])
        .assert()
        .success()
        .stdout(predicates::str::contains("updated remote 'first'"));

    let saved = std::fs::read_to_string(dir.path().join(".quay/config.toml")).unwrap();
    assert!(
        saved.contains("skills-v2.git"),
        "new URL not persisted: {saved}"
    );
    // second remote must still exist.
    assert!(
        saved.contains("skills-second.git"),
        "second remote gone: {saved}"
    );
}

// ── patch provider only ───────────────────────────────────────────────────────

#[test]
fn edit_provider_updates_field() {
    let dir = TempDir::new().unwrap();
    let p = setup_project_with_two_remotes(&dir);

    quay()
        .args([
            "--project",
            &p,
            "remote",
            "edit",
            "first",
            "--provider",
            "gitlab",
        ])
        .assert()
        .success();

    let saved = std::fs::read_to_string(dir.path().join(".quay/config.toml")).unwrap();
    assert!(
        saved.contains("provider = \"gitlab\""),
        "provider not written: {saved}"
    );
}

// ── patch push-mode only ──────────────────────────────────────────────────────

#[test]
fn edit_push_mode_updates_field() {
    let dir = TempDir::new().unwrap();
    let p = setup_project_with_two_remotes(&dir);

    quay()
        .args([
            "--project",
            &p,
            "remote",
            "edit",
            "first",
            "--push-mode",
            "direct",
        ])
        .assert()
        .success();

    let saved = std::fs::read_to_string(dir.path().join(".quay/config.toml")).unwrap();
    assert!(
        saved.contains("push_mode = \"direct\""),
        "push_mode not written: {saved}"
    );
}

// ── patch combined: url + provider + push-mode ────────────────────────────────

#[test]
fn edit_combined_flags_all_apply() {
    let dir = TempDir::new().unwrap();
    let p = setup_project_with_two_remotes(&dir);

    quay()
        .args([
            "--project",
            &p,
            "remote",
            "edit",
            "second",
            "--url",
            "https://gitlab.com/org/skills.git",
            "--provider",
            "gitlab",
            "--push-mode",
            "direct",
        ])
        .assert()
        .success();

    let saved = std::fs::read_to_string(dir.path().join(".quay/config.toml")).unwrap();
    assert!(saved.contains("gitlab.com"), "url: {saved}");
    assert!(saved.contains("provider = \"gitlab\""), "provider: {saved}");
    assert!(
        saved.contains("push_mode = \"direct\""),
        "push_mode: {saved}"
    );
}

// ── --default flips the flag ──────────────────────────────────────────────────

#[test]
fn edit_default_flag_clears_previous_default() {
    let dir = TempDir::new().unwrap();
    let p = setup_project_with_two_remotes(&dir);

    // `first` is currently the default; promoting `second` must clear `first`.
    quay()
        .args(["--project", &p, "remote", "edit", "second", "--default"])
        .assert()
        .success();

    // After listing, only `second` should carry [default].
    quay()
        .args(["--project", &p, "remote", "list"])
        .assert()
        .success()
        .stdout(predicates::str::is_match(r"second\s+\S+\s+\[default\]").unwrap())
        .stdout(
            predicates::str::is_match(r"first\s+\S+\s+\[default\]")
                .unwrap()
                .not(),
        );
}

// ── error: remote not found ───────────────────────────────────────────────────

#[test]
fn edit_unknown_remote_fails() {
    let dir = TempDir::new().unwrap();
    let p = setup_project_with_two_remotes(&dir);

    quay()
        .args([
            "--project",
            &p,
            "remote",
            "edit",
            "ghost",
            "--url",
            "https://example.com/x.git",
        ])
        .assert()
        .failure()
        .stderr(predicates::str::contains("ghost").or(predicates::str::contains("not configured")));
}

// ── error: empty URL ─────────────────────────────────────────────────────────

#[test]
fn edit_empty_url_rejected() {
    let dir = TempDir::new().unwrap();
    let p = setup_project_with_two_remotes(&dir);

    quay()
        .args(["--project", &p, "remote", "edit", "first", "--url", ""])
        .assert()
        .failure()
        .stderr(predicates::str::contains("empty").or(predicates::str::contains("url")));
}

// ── --direct-branch on remote add ─────────────────────────────────────────────

#[test]
fn add_with_direct_branch_persists_field() {
    let dir = TempDir::new().unwrap();
    let p = dir.path().to_str().unwrap().to_string();
    quay().args(["--project", &p, "init"]).assert().success();

    quay()
        .args([
            "--project",
            &p,
            "remote",
            "add",
            "hub",
            "https://github.com/org/skills.git",
            "--push-mode",
            "direct",
            "--direct-branch",
            "develop",
        ])
        .assert()
        .success();

    let saved = std::fs::read_to_string(dir.path().join(".quay/config.toml")).unwrap();
    assert!(
        saved.contains("direct_branch = \"develop\""),
        "direct_branch not written: {saved}"
    );
}

// ── --direct-branch on remote edit ───────────────────────────────────────────

#[test]
fn edit_direct_branch_sets_field() {
    let dir = TempDir::new().unwrap();
    let p = setup_project_with_two_remotes(&dir);

    quay()
        .args([
            "--project",
            &p,
            "remote",
            "edit",
            "first",
            "--direct-branch",
            "develop",
        ])
        .assert()
        .success();

    let saved = std::fs::read_to_string(dir.path().join(".quay/config.toml")).unwrap();
    assert!(
        saved.contains("direct_branch = \"develop\""),
        "direct_branch not written: {saved}"
    );
}

#[test]
fn edit_direct_branch_empty_string_clears_field() {
    let dir = TempDir::new().unwrap();
    let p = dir.path().to_str().unwrap().to_string();
    quay().args(["--project", &p, "init"]).assert().success();

    // Add a remote with direct_branch set.
    quay()
        .args([
            "--project",
            &p,
            "remote",
            "add",
            "hub",
            "https://github.com/org/skills.git",
            "--push-mode",
            "direct",
            "--direct-branch",
            "develop",
        ])
        .assert()
        .success();

    // Now clear it with an empty string.
    quay()
        .args([
            "--project",
            &p,
            "remote",
            "edit",
            "hub",
            "--direct-branch",
            "",
        ])
        .assert()
        .success();

    let saved = std::fs::read_to_string(dir.path().join(".quay/config.toml")).unwrap();
    assert!(
        !saved.contains("direct_branch"),
        "direct_branch should have been cleared, got:\n{saved}"
    );
}

// ── --json output ─────────────────────────────────────────────────────────────

#[test]
fn edit_json_output_shape() {
    let dir = TempDir::new().unwrap();
    let p = setup_project_with_two_remotes(&dir);

    let output = quay()
        .args([
            "--project",
            &p,
            "--json",
            "remote",
            "edit",
            "first",
            "--url",
            "https://github.com/org/new-skills.git",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let text = String::from_utf8_lossy(&output);
    let v: serde_json::Value = serde_json::from_str(&text).expect("valid JSON");
    assert_eq!(v["action"], "edited", "json.action: {text}");
    assert_eq!(v["name"], "first", "json.name: {text}");
    assert!(
        v["url"].as_str().unwrap().contains("new-skills"),
        "json.url: {text}"
    );
}
