//! Integration tests for `quay add` default-interactive-on-TTY behaviour.
//!
//! In CI stdin is a pipe (non-TTY), so bare `quay add` must error with a
//! "name required" message.  The TTY path (picker opens) is verified manually.

use assert_cmd::Command;
use assert_fs::prelude::*;

/// Bare `quay add` in a non-TTY (piped stdin) must exit non-zero and print
/// a message indicating a skill name is required.
#[test]
fn add_bare_non_tty_errors_with_help_message() {
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
        .arg("add")
        .write_stdin("")
        .assert()
        .failure();

    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).to_string();
    assert!(
        stderr.contains("name required") || stderr.contains("-i"),
        "expected 'name required' or '-i' hint in stderr, got: {stderr}"
    );
}
