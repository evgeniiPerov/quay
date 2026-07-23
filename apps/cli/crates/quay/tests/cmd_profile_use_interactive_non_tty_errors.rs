//! Integration tests for `quay profile use -i` non-TTY fallback.
//!
//! Only the non-TTY path is tested in CI because `dialoguer::Select` requires
//! an actual terminal to operate. Manual smoke tests cover the happy path.

use assert_cmd::Command;
use assert_fs::prelude::*;

fn write_user(dir: &assert_fs::TempDir, contents: &str) -> std::path::PathBuf {
    let p = dir.child("user.toml");
    std::fs::write(p.path(), contents).unwrap();
    p.path().to_path_buf()
}

/// When stdin is piped (non-TTY) `quay profile use -i` must exit non-zero and
/// print an error message that mentions "TTY" or "terminal".
#[test]
fn profile_use_interactive_non_tty_exits_with_error() {
    let tmp = assert_fs::TempDir::new().unwrap();
    let user = write_user(
        &tmp,
        r#"
            active_profile = "work"
            [profiles.work.user]
            email = "e@work"
            [profiles.personal.user]
            email = "e@home"
        "#,
    );
    let project = tmp.child("project");
    std::fs::create_dir_all(project.path()).unwrap();

    let mut cmd = Command::cargo_bin("quay").unwrap();
    cmd.args([
        "--user-config",
        user.to_str().unwrap(),
        "--project",
        project.path().to_str().unwrap(),
        "profile",
        "use",
        "-i",
    ])
    .write_stdin("");

    cmd.assert()
        .failure()
        .stderr(predicates::str::is_match("(?i)tty|terminal").unwrap());
}

/// `quay profile use -i` and a positional name must be rejected by clap.
#[test]
fn profile_use_interactive_and_positional_conflict() {
    let tmp = assert_fs::TempDir::new().unwrap();
    let user = write_user(
        &tmp,
        r#"
            active_profile = "work"
            [profiles.work.user]
            email = "e@work"
        "#,
    );
    let project = tmp.child("project");
    std::fs::create_dir_all(project.path()).unwrap();

    Command::cargo_bin("quay")
        .unwrap()
        .args([
            "--user-config",
            user.to_str().unwrap(),
            "--project",
            project.path().to_str().unwrap(),
            "profile",
            "use",
            "-i",
            "work",
        ])
        .assert()
        .failure();
}
