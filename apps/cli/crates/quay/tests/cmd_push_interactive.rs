//! Integration tests for `quay push -i` / `quay push --interactive`.
//!
//! Only the non-TTY fallback path is tested in CI, because `dialoguer::MultiSelect`
//! requires an actual terminal to operate.  Manual smoke tests cover the happy path.

use assert_cmd::Command;
use assert_fs::prelude::*;

/// When stdin is piped (non-TTY) `quay push -i` must exit non-zero and print a
/// clear error mentioning "TTY" or "terminal".
#[test]
fn push_interactive_non_tty_exits_with_error() {
    let project = assert_fs::TempDir::new().unwrap();
    // Write a minimal quay config so the command doesn't fail on "no remotes" first.
    project
        .child(".quay/config.toml")
        .write_str(
            r#"[remotes.hub]
url = "https://example.com/hub.git"
default = true
"#,
        )
        .unwrap();
    // Create at least one local skill so we get past the "no skills" early-return.
    project
        .child(".agents/skills/my-skill/SKILL.md")
        .write_str("---\nname: my-skill\ndescription: d\n---\n")
        .unwrap();

    let mut cmd = Command::cargo_bin("quay").unwrap();
    cmd.arg("--project")
        .arg(project.path())
        .arg("push")
        .arg("-i")
        // Pipe /dev/null as stdin — ensures the process sees a non-TTY.
        .pipe_stdin("/dev/null")
        .unwrap();

    cmd.assert()
        .failure()
        .stderr(predicates::str::is_match("(?i)tty|terminal").unwrap());
}

/// `quay push --interactive` (long form) produces the same non-TTY error.
#[test]
fn push_interactive_long_flag_non_tty_exits_with_error() {
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
    project
        .child(".agents/skills/alpha/SKILL.md")
        .write_str("---\nname: alpha\ndescription: a\n---\n")
        .unwrap();

    let mut cmd = Command::cargo_bin("quay").unwrap();
    cmd.arg("--project")
        .arg(project.path())
        .arg("push")
        .arg("--interactive")
        .pipe_stdin("/dev/null")
        .unwrap();

    cmd.assert()
        .failure()
        .stderr(predicates::str::is_match("(?i)tty|terminal").unwrap());
}

/// `quay push -i` and a positional skill name must be rejected by clap
/// (they are declared `conflicts_with`).
#[test]
fn push_interactive_and_positional_conflict() {
    let project = assert_fs::TempDir::new().unwrap();

    let mut cmd = Command::cargo_bin("quay").unwrap();
    cmd.arg("--project")
        .arg(project.path())
        .arg("push")
        .arg("-i")
        .arg("my-skill");

    // clap exits 2 for usage errors.
    cmd.assert().failure();
}
