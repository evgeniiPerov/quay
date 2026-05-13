//! Integration tests for `quay profile edit`.

use assert_cmd::Command;
use assert_fs::prelude::*;
use predicates::prelude::*;

// ── helpers ───────────────────────────────────────────────────────────────────

fn write_user(dir: &assert_fs::TempDir, contents: &str) -> std::path::PathBuf {
    let p = dir.child("user.toml");
    std::fs::write(p.path(), contents).unwrap();
    p.path().to_path_buf()
}

fn two_profile_config(dir: &assert_fs::TempDir) -> std::path::PathBuf {
    write_user(
        dir,
        r#"
active_profile = "work"
[profiles.work.user]
email = "e@work"
[profiles.personal.user]
email = "e@home"
"#,
    )
}

fn quay_edit(cfg: &std::path::Path, args: &[&str]) -> assert_cmd::assert::Assert {
    let mut cmd = Command::cargo_bin("quay").unwrap();
    cmd.args(["--user-config", cfg.to_str().unwrap()]);
    cmd.args(["profile", "edit"]);
    cmd.args(args);
    cmd.assert()
}

// ── explicit --email ──────────────────────────────────────────────────────────

#[test]
fn edit_email_updates_on_disk() {
    let tmp = assert_fs::TempDir::new().unwrap();
    let cfg = two_profile_config(&tmp);

    quay_edit(&cfg, &["work", "--email", "new@work"])
        .success()
        .stdout(predicates::str::contains("updated profile 'work'"));

    let saved = std::fs::read_to_string(&cfg).unwrap();
    assert!(saved.contains("new@work"), "email not updated: {saved}");
    // personal should be untouched.
    assert!(saved.contains("e@home"), "personal removed: {saved}");
}

#[test]
fn edit_email_rejects_unknown_profile() {
    let tmp = assert_fs::TempDir::new().unwrap();
    let cfg = two_profile_config(&tmp);

    quay_edit(&cfg, &["ghost", "--email", "x@y"])
        .failure()
        .stderr(predicates::str::contains("ghost").or(predicates::str::contains("unknown")));
}

#[test]
fn edit_no_flags_returns_error() {
    let tmp = assert_fs::TempDir::new().unwrap();
    let cfg = two_profile_config(&tmp);

    quay_edit(&cfg, &["work"])
        .failure()
        .stderr(predicates::str::contains("nothing to do"));
}

// ── --from-toml ───────────────────────────────────────────────────────────────

#[test]
fn edit_from_toml_file_replaces_email_and_remotes() {
    let tmp = assert_fs::TempDir::new().unwrap();
    let cfg = two_profile_config(&tmp);

    let toml_file = tmp.child("edit.toml");
    toml_file
        .write_str(
            r#"
email = "replaced@work"
[remotes.hub]
url = "https://github.com/org/skills.git"
default = true
"#,
        )
        .unwrap();

    quay_edit(
        &cfg,
        &["work", "--from-toml", toml_file.path().to_str().unwrap()],
    )
    .success();

    let saved = std::fs::read_to_string(&cfg).unwrap();
    assert!(
        saved.contains("replaced@work"),
        "email not replaced: {saved}"
    );
    assert!(
        saved.contains("[profiles.work.remotes.hub]"),
        "remote not written: {saved}"
    );
    // personal must still exist.
    assert!(saved.contains("e@home"), "personal stripped: {saved}");
}

#[test]
fn edit_from_toml_via_stdin_replaces_content() {
    let tmp = assert_fs::TempDir::new().unwrap();
    let cfg = two_profile_config(&tmp);

    let toml = r#"
email = "stdin@work"
[remotes.stdin-hub]
url = "https://github.com/org/stdin-skills.git"
default = true
"#;

    Command::cargo_bin("quay")
        .unwrap()
        .args([
            "--user-config",
            cfg.to_str().unwrap(),
            "profile",
            "edit",
            "work",
            "--from-toml",
            "-",
        ])
        .write_stdin(toml)
        .assert()
        .success();

    let saved = std::fs::read_to_string(&cfg).unwrap();
    assert!(saved.contains("stdin@work"), "email not set: {saved}");
    assert!(saved.contains("stdin-hub"), "remote not written: {saved}");
}

#[test]
fn edit_from_toml_and_interactive_are_mutually_exclusive() {
    let tmp = assert_fs::TempDir::new().unwrap();
    let cfg = two_profile_config(&tmp);

    Command::cargo_bin("quay")
        .unwrap()
        .args([
            "--user-config",
            cfg.to_str().unwrap(),
            "profile",
            "edit",
            "work",
            "-i",
            "--from-toml",
            "-",
        ])
        .write_stdin("")
        .assert()
        .failure();
}

#[test]
fn edit_from_toml_and_email_are_mutually_exclusive() {
    let tmp = assert_fs::TempDir::new().unwrap();
    let cfg = two_profile_config(&tmp);

    Command::cargo_bin("quay")
        .unwrap()
        .args([
            "--user-config",
            cfg.to_str().unwrap(),
            "profile",
            "edit",
            "work",
            "--from-toml",
            "-",
            "--email",
            "x@y",
        ])
        .write_stdin("")
        .assert()
        .failure();
}

// ── interactive (-i) in non-TTY ───────────────────────────────────────────────

#[test]
fn edit_interactive_non_tty_errors_with_clear_message() {
    let tmp = assert_fs::TempDir::new().unwrap();
    let cfg = two_profile_config(&tmp);

    let assert = Command::cargo_bin("quay")
        .unwrap()
        .args([
            "--user-config",
            cfg.to_str().unwrap(),
            "profile",
            "edit",
            "work",
            "-i",
        ])
        .write_stdin("") // force non-TTY stdin
        .assert()
        .failure();

    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).to_string();
    assert!(
        stderr.to_lowercase().contains("interactive")
            || stderr.to_lowercase().contains("tty")
            || stderr.to_lowercase().contains("terminal"),
        "error should mention interactive/TTY/terminal: {stderr}"
    );
}

#[test]
fn edit_interactive_email_are_mutually_exclusive() {
    let tmp = assert_fs::TempDir::new().unwrap();
    let cfg = two_profile_config(&tmp);

    Command::cargo_bin("quay")
        .unwrap()
        .args([
            "--user-config",
            cfg.to_str().unwrap(),
            "profile",
            "edit",
            "work",
            "-i",
            "--email",
            "x@y",
        ])
        .write_stdin("")
        .assert()
        .failure();
}

// ── --json output ─────────────────────────────────────────────────────────────

#[test]
fn edit_json_output_shape() {
    let tmp = assert_fs::TempDir::new().unwrap();
    let cfg = two_profile_config(&tmp);

    let output = Command::cargo_bin("quay")
        .unwrap()
        .args([
            "--user-config",
            cfg.to_str().unwrap(),
            "--json",
            "profile",
            "edit",
            "work",
            "--email",
            "json@work",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let text = String::from_utf8_lossy(&output);
    let v: serde_json::Value = serde_json::from_str(&text).expect("valid JSON");
    assert_eq!(v["profile"], "work", "json.profile: {text}");
    assert_eq!(v["email"], "json@work", "json.email: {text}");
}

// ── regression guards ─────────────────────────────────────────────────────────

/// Regression: edit-via-TOML used to hardcode `direct_branch: None`, silently
/// discarding the field even when the input TOML set it. Verifies that a
/// `direct_branch = "develop"` round-trips through `quay profile edit
/// --from-toml`.
#[test]
fn edit_from_toml_preserves_direct_branch() {
    let tmp = assert_fs::TempDir::new().unwrap();
    let cfg = two_profile_config(&tmp);

    let toml = r#"
email = "e@work"
[remotes.hub]
url = "https://dev.azure.com/example-org/proj/_git/repo"
provider = "azuredevops"
push_mode = "direct"
direct_branch = "develop"
default = true
"#;
    let toml_file = tmp.child("edit.toml");
    toml_file.write_str(toml).unwrap();

    quay_edit(
        &cfg,
        &["work", "--from-toml", toml_file.path().to_str().unwrap()],
    )
    .success();

    let saved = std::fs::read_to_string(&cfg).unwrap();
    assert!(
        saved.contains("direct_branch = \"develop\""),
        "direct_branch was dropped on disk: {saved}"
    );
}

/// Regression: `quay profile edit X --email <bad>` used to accept anything
/// non-empty. Now must reject inputs missing `@` or containing whitespace.
#[test]
fn edit_email_rejects_invalid_format() {
    let tmp = assert_fs::TempDir::new().unwrap();
    let cfg = two_profile_config(&tmp);

    quay_edit(&cfg, &["work", "--email", "no-at-sign"])
        .failure()
        .stderr(predicate::str::contains("email must contain '@'"));

    quay_edit(&cfg, &["work", "--email", "has space@x.com"])
        .failure()
        .stderr(predicate::str::contains("email must not contain whitespace"));
}

/// Regression: edit-via-TOML used to skip email validation; ingesting a TOML
/// with `email = "no at sign"` silently wrote garbage to disk.
#[test]
fn edit_from_toml_rejects_invalid_email() {
    let tmp = assert_fs::TempDir::new().unwrap();
    let cfg = two_profile_config(&tmp);

    let toml = r#"email = "no-at-sign""#;
    let toml_file = tmp.child("edit.toml");
    toml_file.write_str(toml).unwrap();

    quay_edit(
        &cfg,
        &["work", "--from-toml", toml_file.path().to_str().unwrap()],
    )
    .failure()
    .stderr(predicate::str::contains("email must contain '@'"));
}
