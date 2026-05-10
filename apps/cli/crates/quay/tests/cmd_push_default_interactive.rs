//! Integration tests for `quay push` default-interactive-on-TTY behaviour.

use assert_cmd::Command;
use assert_fs::prelude::*;

/// Bare `quay push` in a non-TTY (piped stdin) must exit non-zero and print
/// a message indicating a skill name is required.
#[test]
fn push_bare_non_tty_errors_with_help_message() {
    let project = assert_fs::TempDir::new().unwrap();
    project
        .child(".quay/config.toml")
        .write_str(
            r#"[remotes.hub]
url = "https://example.com/hub.git"
default = true
"#,
        )
        .unwrap();

    let assert = Command::cargo_bin("quay")
        .unwrap()
        .arg("--project")
        .arg(project.path())
        .arg("push")
        .write_stdin("")
        .assert()
        .failure();

    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).to_string();
    assert!(
        stderr.contains("name required") || stderr.contains("-i"),
        "expected 'name required' or '-i' hint in stderr, got: {stderr}"
    );
}
