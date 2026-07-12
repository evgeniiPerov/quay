//! Discovery-driven `quay link` reconcile: unmanaged mirror roots that were
//! never explicitly configured (e.g. a hand-created `.codex/skills` dir) are
//! discovered, classified, and either adopted, flagged as diverged, or
//! flagged as needing an opt-in decision.

use assert_cmd::Command;
use assert_fs::prelude::*;

fn make_project_with_skill(dir: &assert_fs::TempDir, skill: &str) {
    let p = dir.path().to_str().unwrap();
    Command::cargo_bin("quay")
        .unwrap()
        .args(["--project", p, "init"])
        .assert()
        .success();
    let skill_dir = dir.child(format!(".agents/skills/{}", skill));
    std::fs::create_dir_all(skill_dir.path()).unwrap();
    std::fs::write(
        skill_dir.path().join("SKILL.md"),
        b"---\nname: x\ndescription: y\nversion: 0.1.0\n---\nbody\n",
    )
    .unwrap();
}

#[test]
fn link_check_reports_diverged_unmanaged_dir_and_exits_nonzero() {
    let dir = assert_fs::TempDir::new().unwrap();
    make_project_with_skill(&dir, "foo");

    // Unmanaged, unconfigured `.codex` mirror someone hand-edited with
    // content that differs from canonical.
    let codex_skill = dir.child(".codex/skills/foo/SKILL.md");
    codex_skill.write_str("HAND EDITED DIFFERENT").unwrap();

    let p = dir.path().to_str().unwrap();
    Command::cargo_bin("quay")
        .unwrap()
        .args(["--project", p, "link", "check"])
        .assert()
        .failure()
        .stderr(predicates::str::contains("content differs from canonical"));

    // The hand-edited file must be preserved, untouched.
    codex_skill.assert("HAND EDITED DIFFERENT");
}

#[test]
fn link_json_noninteractive_does_not_adopt_or_write_config() {
    let dir = assert_fs::TempDir::new().unwrap();
    make_project_with_skill(&dir, "foo");

    // Unmanaged `.codex` mirror identical to canonical (adoptable), but
    // `auto_link` is unset and we are running non-interactively (--json).
    let canonical_body = b"---\nname: x\ndescription: y\nversion: 0.1.0\n---\nbody\n";
    let codex_dir = dir.child(".codex/skills/foo");
    std::fs::create_dir_all(codex_dir.path()).unwrap();
    std::fs::write(codex_dir.path().join("SKILL.md"), canonical_body).unwrap();

    let p = dir.path().to_str().unwrap();
    Command::cargo_bin("quay")
        .unwrap()
        .args(["--project", p, "link", "--json"])
        .assert()
        .code(predicates::ord::ne(0)); // needs-optin, unresolved -> non-zero

    // Still a real dir, not symlinked.
    assert!(
        std::fs::symlink_metadata(dir.path().join(".codex/skills/foo"))
            .unwrap()
            .file_type()
            .is_dir()
    );

    // Config must not gain an `auto_link` key non-interactively.
    let cfg = std::fs::read_to_string(dir.child(".quay/config.toml").path()).unwrap();
    assert!(
        !cfg.contains("auto_link"),
        "config must not be written non-interactively:\n{}",
        cfg
    );
}

/// Set `install.auto_link = <value>` in the project config by reading and
/// rewriting the TOML document (mirrors `append_mirror_to_config` in
/// `cmd_link.rs`, avoiding duplicate `[install]` tables).
fn set_auto_link(dir: &assert_fs::TempDir, value: bool) {
    let cfg_path = dir.child(".quay/config.toml").path().to_path_buf();
    let text = std::fs::read_to_string(&cfg_path).unwrap();
    let mut doc: toml::Value = toml::from_str(&text).unwrap();
    let install = doc
        .get_mut("install")
        .expect("init must have written [install]");
    install
        .as_table_mut()
        .unwrap()
        .insert("auto_link".into(), toml::Value::Boolean(value));
    std::fs::write(&cfg_path, toml::to_string_pretty(&doc).unwrap()).unwrap();
}

/// When the user has already opted out (`auto_link = false`), an unmanaged
/// but adoptable dir is an accepted, reported state — not a failure. `quay
/// link` must exit 0 and must NOT adopt/symlink the dir.
#[test]
fn link_exits_zero_when_needs_optin_and_auto_link_explicitly_false() {
    let dir = assert_fs::TempDir::new().unwrap();
    make_project_with_skill(&dir, "foo");
    set_auto_link(&dir, false);

    // Unmanaged `.codex` mirror identical to canonical (adoptable).
    let canonical_body = b"---\nname: x\ndescription: y\nversion: 0.1.0\n---\nbody\n";
    let codex_dir = dir.child(".codex/skills/foo");
    std::fs::create_dir_all(codex_dir.path()).unwrap();
    std::fs::write(codex_dir.path().join("SKILL.md"), canonical_body).unwrap();

    let p = dir.path().to_str().unwrap();
    Command::cargo_bin("quay")
        .unwrap()
        .args(["--project", p, "link"])
        .assert()
        .success();

    // Still a real dir, not adopted/symlinked.
    assert!(
        std::fs::symlink_metadata(dir.path().join(".codex/skills/foo"))
            .unwrap()
            .file_type()
            .is_dir()
    );
}

/// `quay link check` must respect the `auto_link = false` opt-out: an
/// adoptable-but-unmanaged dir is an accepted state, not drift, so `check`
/// must exit 0 (not just plain `quay link`).
#[test]
fn link_check_exits_zero_when_adoptable_and_opted_out() {
    let dir = assert_fs::TempDir::new().unwrap();
    make_project_with_skill(&dir, "foo");
    set_auto_link(&dir, false);

    // Unmanaged `.codex` mirror identical to canonical (adoptable).
    let canonical_body = b"---\nname: x\ndescription: y\nversion: 0.1.0\n---\nbody\n";
    let codex_dir = dir.child(".codex/skills/foo");
    std::fs::create_dir_all(codex_dir.path()).unwrap();
    std::fs::write(codex_dir.path().join("SKILL.md"), canonical_body).unwrap();

    let p = dir.path().to_str().unwrap();
    Command::cargo_bin("quay")
        .unwrap()
        .args(["--project", p, "link", "check"])
        .assert()
        .success();
}

/// When `auto_link = true` is already set in config (not a first-time
/// interactive opt-in), a discovered adoptable mirror must still be adopted
/// AND registered in `[install].mirrors` for future runs.
#[test]
fn link_registers_adopted_mirror_when_auto_link_true() {
    let dir = assert_fs::TempDir::new().unwrap();
    make_project_with_skill(&dir, "foo");
    set_auto_link(&dir, true);

    let canonical_body = b"---\nname: x\ndescription: y\nversion: 0.1.0\n---\nbody\n";
    let codex_dir = dir.child(".codex/skills/foo");
    std::fs::create_dir_all(codex_dir.path()).unwrap();
    std::fs::write(codex_dir.path().join("SKILL.md"), canonical_body).unwrap();

    let p = dir.path().to_str().unwrap();
    Command::cargo_bin("quay")
        .unwrap()
        .args(["--project", p, "link"])
        .assert()
        .success();

    assert!(
        std::fs::symlink_metadata(dir.path().join(".codex/skills/foo"))
            .unwrap()
            .file_type()
            .is_symlink(),
        "adoptable dir must be converted to a symlink"
    );

    let cfg = std::fs::read_to_string(dir.child(".quay/config.toml").path()).unwrap();
    assert!(
        cfg.contains(".codex/skills"),
        "adopted mirror must be registered in [install].mirrors, got:\n{}",
        cfg
    );
}

/// Adds a mirror entry to the project config by reading and rewriting it via
/// the raw TOML document, mirroring `cmd_link.rs`'s helper of the same name.
fn append_mirror_to_config(dir: &assert_fs::TempDir, mirror_path: &str, strategy: &str) {
    let cfg_path = dir.child(".quay/config.toml").path().to_path_buf();
    let text = std::fs::read_to_string(&cfg_path).unwrap();
    let mut doc: toml::Value = toml::from_str(&text).unwrap();

    let mirrors = doc
        .get_mut("install")
        .and_then(|i| i.get_mut("mirrors"))
        .expect("init must have written [install] with mirrors");

    let entry = toml::Value::Table({
        let mut t = toml::map::Map::new();
        t.insert("path".into(), toml::Value::String(mirror_path.into()));
        t.insert("strategy".into(), toml::Value::String(strategy.into()));
        t
    });

    if let toml::Value::Array(arr) = mirrors {
        arr.push(entry);
    }

    std::fs::write(&cfg_path, toml::to_string_pretty(&doc).unwrap()).unwrap();
}

/// `quay link check` must be read-only: a configured mirror that was never
/// applied (missing on disk) is reported as drift, but `check` must not
/// create it as a side effect.
#[test]
fn link_check_does_not_create_missing_configured_mirror() {
    let dir = assert_fs::TempDir::new().unwrap();
    make_project_with_skill(&dir, "foo");
    append_mirror_to_config(&dir, ".claude/skills", "symlink");

    let p = dir.path().to_str().unwrap();
    Command::cargo_bin("quay")
        .unwrap()
        .args(["--project", p, "link", "check"])
        .assert()
        .failure();

    // No side-effect symlink/dir must have been created by `check`.
    assert!(
        !dir.path().join(".claude/skills/foo").exists(),
        "quay link check must not create the missing mirror as a side effect"
    );
}
