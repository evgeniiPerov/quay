//! Smoke test: verify `quay tui --help` succeeds after CreatePush screen
//! wire-up. This guards against regressions in argument parsing dispatch.

use assert_cmd::Command;

#[test]
fn tui_help_still_succeeds() {
    Command::cargo_bin("quay")
        .unwrap()
        .args(["tui", "--help"])
        .assert()
        .success();
}
