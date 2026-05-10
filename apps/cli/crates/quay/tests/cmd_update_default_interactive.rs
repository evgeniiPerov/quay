//! Integration tests for `quay update` default-interactive-on-TTY behaviour.
//!
//! Key contract: bare `quay update` in non-TTY must still update all
//! (backward-compatible).  `--all` bypasses the picker even on a TTY.

use assert_cmd::Command;
use assert_fs::prelude::*;

/// Bare `quay update` in non-TTY (piped stdin) succeeds and behaves like
/// "update all" — i.e. exits 0 even when nothing is installed.
#[test]
fn update_bare_non_tty_succeeds_as_update_all() {
    let project = assert_fs::TempDir::new().unwrap();
    project
        .child(".quay/config.toml")
        .write_str("[remotes]\n")
        .unwrap();

    Command::cargo_bin("quay")
        .unwrap()
        .arg("--project")
        .arg(project.path())
        .arg("update")
        .write_stdin("")
        .assert()
        .success();
}

/// `quay update --all` in non-TTY also succeeds (no skills installed → nothing to do).
#[test]
fn update_all_flag_succeeds_non_tty() {
    let project = assert_fs::TempDir::new().unwrap();
    project
        .child(".quay/config.toml")
        .write_str("[remotes]\n")
        .unwrap();

    Command::cargo_bin("quay")
        .unwrap()
        .arg("--project")
        .arg(project.path())
        .arg("update")
        .arg("--all")
        .write_stdin("")
        .assert()
        .success();
}
