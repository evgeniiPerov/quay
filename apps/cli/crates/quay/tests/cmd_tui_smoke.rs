//! Smoke test: verify `quay tui --help` succeeds. Full keyboard-driven TUI
//! tests are exercised by per-screen TestBackend tests in `quay-cli`.

use assert_cmd::Command;

#[test]
fn tui_subcommand_help_succeeds() {
    Command::cargo_bin("quay")
        .unwrap()
        .args(["tui", "--help"])
        .assert()
        .success();
}
