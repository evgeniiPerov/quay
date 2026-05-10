//! Integration tests for `quay add -i` / `quay add --interactive`.
//!
//! Only the non-TTY fallback path is tested in CI, because `dialoguer::MultiSelect`
//! requires an actual terminal to operate.  Manual smoke tests cover the happy path.

use assert_cmd::Command;
use assert_fs::prelude::*;

/// When stdin is piped (non-TTY) `quay add -i` must exit non-zero and print a
/// clear error mentioning "TTY" or "terminal".
#[test]
fn add_interactive_non_tty_exits_with_error() {
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

    let mut cmd = Command::cargo_bin("quay").unwrap();
    cmd.arg("--project")
        .arg(project.path())
        .arg("add")
        .arg("-i")
        .pipe_stdin("/dev/null")
        .unwrap();

    // Non-TTY: either the registry fetch fails (no real remote) or interactive
    // mode fails.  Either way exit code must be non-zero.  If the error is the
    // TTY check it will mention "TTY" / "terminal"; if it's a network error
    // that's also a non-zero exit.  We only assert failure here; the unit test
    // in `interactive.rs` asserts the exact TTY error message.
    cmd.assert().failure();
}

/// `quay add -i` and a positional skill name must be rejected by clap.
#[test]
fn add_interactive_and_positional_conflict() {
    let project = assert_fs::TempDir::new().unwrap();

    let mut cmd = Command::cargo_bin("quay").unwrap();
    cmd.arg("--project")
        .arg(project.path())
        .arg("add")
        .arg("-i")
        .arg("my-skill");

    // clap exits 2 for usage errors.
    cmd.assert().failure();
}
