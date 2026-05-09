//! Integration tests for the `quay tui --check-config-only` probe flag.
//!
//! The probe exits 2 when onboarding is needed, 0 when it is not.
//! `HOME` is overridden so the default config path (`$HOME/.config/quay/config.toml`)
//! resolves inside a temporary directory, simulating a fresh or configured install.

use assert_cmd::Command;
use assert_fs::prelude::*;

/// A missing config file triggers onboarding (exit 2).
#[test]
fn missing_config_triggers_onboarding() {
    let home = assert_fs::TempDir::new().unwrap();
    Command::cargo_bin("quay")
        .unwrap()
        .env("HOME", home.path())
        .args(["tui", "--check-config-only"])
        .assert()
        .code(2);
}

/// `onboarded = true` with no profiles still triggers onboarding (exit 2).
///
/// This is the recovery path for the bug where a user who skipped onboarding
/// once (writing `onboarded = true` with no profiles) would be stuck — the
/// gate now uses `profiles.is_empty()` alone, not the `onboarded` flag.
#[test]
fn onboarded_marker_without_profiles_still_triggers_onboarding() {
    let home = assert_fs::TempDir::new().unwrap();
    let config_dir = home.child(".config/quay");
    config_dir.create_dir_all().unwrap();
    config_dir
        .child("config.toml")
        .write_str("[meta]\nonboarded = true\n")
        .unwrap();
    Command::cargo_bin("quay")
        .unwrap()
        .env("HOME", home.path())
        .args(["tui", "--check-config-only"])
        .assert()
        .code(2);
}

/// A config with at least one profile skips onboarding (exit 0).
#[test]
fn profiles_present_skips_onboarding() {
    let home = assert_fs::TempDir::new().unwrap();
    let config_dir = home.child(".config/quay");
    config_dir.create_dir_all().unwrap();
    config_dir
        .child("config.toml")
        .write_str("[meta]\nonboarded = true\n\n[profiles.personal.user]\nname = \"Test\"\n")
        .unwrap();
    Command::cargo_bin("quay")
        .unwrap()
        .env("HOME", home.path())
        .args(["tui", "--check-config-only"])
        .assert()
        .code(0);
}
