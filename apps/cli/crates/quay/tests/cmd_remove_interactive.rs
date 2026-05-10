//! Integration tests for `quay remove -i` / `quay remove --interactive`.

use assert_cmd::Command;
use assert_fs::prelude::*;

/// When stdin is piped (non-TTY) `quay remove -i` must exit non-zero and print a
/// clear error mentioning "interactive" or "TTY".
#[test]
fn remove_interactive_non_tty_exits_with_error() {
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
        .arg("-i")
        .write_stdin("")
        .assert()
        .failure();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).to_string();
    assert!(
        stderr.contains("interactive") || stderr.contains("TTY") || stderr.contains("terminal"),
        "expected TTY/interactive error, got: {stderr}"
    );
}

/// `quay remove -i` and a positional skill name must be rejected by clap.
#[test]
fn remove_interactive_and_positional_conflict() {
    let project = assert_fs::TempDir::new().unwrap();

    // clap exits 2 for usage errors.
    Command::cargo_bin("quay")
        .unwrap()
        .arg("--project")
        .arg(project.path())
        .arg("remove")
        .arg("-i")
        .arg("foo")
        .assert()
        .failure();
}
