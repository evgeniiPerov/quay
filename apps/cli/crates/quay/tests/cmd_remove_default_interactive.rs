//! Integration tests for `quay remove` default-interactive-on-TTY behaviour.

use assert_cmd::Command;
use assert_fs::prelude::*;

/// Bare `quay remove` in non-TTY (piped stdin) must exit non-zero with a
/// message indicating a skill name is required.
#[test]
fn remove_bare_non_tty_errors_with_help_message() {
    let project = assert_fs::TempDir::new().unwrap();
    project
        .child(".quay/config.toml")
        .write_str("[remotes]\n")
        .unwrap();

    let assert = Command::cargo_bin("quay")
        .unwrap()
        .arg("--project")
        .arg(project.path())
        .arg("remove")
        .write_stdin("")
        .assert()
        .failure();

    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).to_string();
    assert!(
        stderr.contains("name required") || stderr.contains("-i"),
        "expected 'name required' or '-i' hint in stderr, got: {stderr}"
    );
}
