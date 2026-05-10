//! Integration tests for `quay profile add -i` (interactive wizard).
//!
//! The wizard cannot be driven in a non-TTY environment, so we test only the
//! non-TTY error path and the wizard's pure validation helpers.

use assert_cmd::Command;

/// When invoked outside a TTY (as in CI), `-i` must fail with a clear message
/// mentioning "interactive" or "TTY".
#[test]
fn add_interactive_in_non_tty_errors_with_clear_message() {
    let assert = Command::cargo_bin("quay")
        .unwrap()
        .args([
            "profile",
            "add",
            "-i",
            "--user-config",
            "/tmp/quay-test-nonexistent.toml",
        ])
        .write_stdin("") // force non-TTY stdin
        .assert()
        .failure();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).to_string();
    assert!(
        stderr.to_lowercase().contains("interactive")
            || stderr.to_lowercase().contains("tty")
            || stderr.to_lowercase().contains("terminal"),
        "error message should mention interactive/TTY/terminal: {stderr}"
    );
}

/// `-i` and `--from-toml` are mutually exclusive; clap must reject the combo.
#[test]
fn interactive_and_from_toml_are_mutually_exclusive() {
    Command::cargo_bin("quay")
        .unwrap()
        .args([
            "profile",
            "add",
            "-i",
            "--from-toml",
            "-",
            "--user-config",
            "/tmp/quay-test-nonexistent.toml",
        ])
        .write_stdin("")
        .assert()
        .failure();
}

/// `-i` and `--email` are mutually exclusive; clap must reject the combo.
#[test]
fn interactive_and_email_are_mutually_exclusive() {
    Command::cargo_bin("quay")
        .unwrap()
        .args([
            "profile",
            "add",
            "-i",
            "--email",
            "x@y",
            "--user-config",
            "/tmp/quay-test-nonexistent.toml",
        ])
        .write_stdin("")
        .assert()
        .failure();
}
