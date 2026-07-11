//! `quay add` / `list` / `info` / `remove` against a real local bare-repo hub.
//! Quay fetches registries via `git clone` (see `CloneFetcher`), so a
//! filesystem bare repo is the deterministic, offline way to exercise these
//! commands end-to-end — no HTTP mocking involved.

use assert_cmd::Command;
use assert_fs::prelude::*;
use std::path::Path;

fn git(dir: &Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "T")
        .env("GIT_AUTHOR_EMAIL", "t@e")
        .env("GIT_COMMITTER_NAME", "T")
        .env("GIT_COMMITTER_EMAIL", "t@e")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

const REGISTRY: &str = r#"{
    "hub": "fixture",
    "generated_at": "2026-05-08T00:00:00Z",
    "schema_version": 1,
    "skills": {
        "csv-parse": {
            "version": "1.0.0",
            "description": "Parse CSV.",
            "tags": [],
            "path": "skills/csv-parse",
            "sha": "deadbeef",
            "files": ["SKILL.md"]
        }
    }
}"#;

const SKILL_MD: &str = "---\nname: csv-parse\ndescription: Parse CSV.\nversion: 1.0.0\n---\nbody\n";

/// Builds a local bare-repo hub whose `registry.json` advertises `csv-parse`,
/// with `SKILL.md` at the advertised `path`. Returns the bare repo path.
fn seed_hub(tmp: &Path) -> std::path::PathBuf {
    let bare = tmp.join("hub.git");
    git(
        tmp,
        &["init", "--bare", "-b", "main", bare.to_str().unwrap()],
    );
    let work = tmp.join("hub-work");
    git(
        tmp,
        &["clone", bare.to_str().unwrap(), work.to_str().unwrap()],
    );
    std::fs::write(work.join("registry.json"), REGISTRY).unwrap();
    let skill_dir = work.join("skills/csv-parse");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(skill_dir.join("SKILL.md"), SKILL_MD).unwrap();
    git(&work, &["add", "-A"]);
    git(&work, &["commit", "-m", "seed registry"]);
    git(&work, &["push", "origin", "main"]);
    bare
}

/// Writes an empty user config and returns `(XDG_CONFIG_HOME, user_config_path)`
/// so tests never pick up the host's real `~/.config/quay`.
fn isolated_user_config(tmp: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
    let cfg_home = tmp.join("cfg");
    let user_cfg = cfg_home.join("quay/config.toml");
    std::fs::create_dir_all(user_cfg.parent().unwrap()).unwrap();
    std::fs::write(&user_cfg, "").unwrap();
    (cfg_home, user_cfg)
}

#[test]
fn quay_add_installs_skill_from_bare_hub() {
    let tmp = assert_fs::TempDir::new().unwrap();
    let bare = seed_hub(tmp.path());
    let (cfg_home, user_cfg) = isolated_user_config(tmp.path());

    let dir = tmp.child("project");
    std::fs::create_dir_all(dir.path()).unwrap();
    let p = dir.path().to_str().unwrap();
    let bare_url = bare.to_str().unwrap();

    Command::cargo_bin("quay")
        .unwrap()
        .env("XDG_CONFIG_HOME", &cfg_home)
        .args([
            "--project",
            p,
            "--user-config",
            user_cfg.to_str().unwrap(),
            "init",
        ])
        .assert()
        .success();
    Command::cargo_bin("quay")
        .unwrap()
        .env("XDG_CONFIG_HOME", &cfg_home)
        .args([
            "--project",
            p,
            "--user-config",
            user_cfg.to_str().unwrap(),
            "remote",
            "add",
            "h",
            bare_url,
            "--default",
        ])
        .assert()
        .success();
    Command::cargo_bin("quay")
        .unwrap()
        .env("XDG_CONFIG_HOME", &cfg_home)
        .args([
            "--project",
            p,
            "--user-config",
            user_cfg.to_str().unwrap(),
            "add",
            "csv-parse",
        ])
        .assert()
        .success();

    dir.child(".agents/skills/csv-parse/SKILL.md")
        .assert(predicates::path::exists());
}

#[test]
fn quay_list_shows_installed_skill() {
    let tmp = assert_fs::TempDir::new().unwrap();
    let bare = seed_hub(tmp.path());
    let (cfg_home, user_cfg) = isolated_user_config(tmp.path());

    let dir = tmp.child("project");
    std::fs::create_dir_all(dir.path()).unwrap();
    let p = dir.path().to_str().unwrap();
    let bare_url = bare.to_str().unwrap();

    Command::cargo_bin("quay")
        .unwrap()
        .env("XDG_CONFIG_HOME", &cfg_home)
        .args([
            "--project",
            p,
            "--user-config",
            user_cfg.to_str().unwrap(),
            "init",
        ])
        .assert()
        .success();
    Command::cargo_bin("quay")
        .unwrap()
        .env("XDG_CONFIG_HOME", &cfg_home)
        .args([
            "--project",
            p,
            "--user-config",
            user_cfg.to_str().unwrap(),
            "remote",
            "add",
            "h",
            bare_url,
            "--default",
        ])
        .assert()
        .success();
    Command::cargo_bin("quay")
        .unwrap()
        .env("XDG_CONFIG_HOME", &cfg_home)
        .args([
            "--project",
            p,
            "--user-config",
            user_cfg.to_str().unwrap(),
            "add",
            "csv-parse",
        ])
        .assert()
        .success();
    Command::cargo_bin("quay")
        .unwrap()
        .env("XDG_CONFIG_HOME", &cfg_home)
        .args([
            "--project",
            p,
            "--user-config",
            user_cfg.to_str().unwrap(),
            "list",
        ])
        .assert()
        .success()
        .stdout(predicates::str::contains("csv-parse"))
        .stdout(predicates::str::contains("1.0.0"));
}

#[test]
fn quay_remove_uninstalls_skill() {
    let tmp = assert_fs::TempDir::new().unwrap();
    let bare = seed_hub(tmp.path());
    let (cfg_home, user_cfg) = isolated_user_config(tmp.path());

    let dir = tmp.child("project");
    std::fs::create_dir_all(dir.path()).unwrap();
    let p = dir.path().to_str().unwrap();
    let bare_url = bare.to_str().unwrap();

    Command::cargo_bin("quay")
        .unwrap()
        .env("XDG_CONFIG_HOME", &cfg_home)
        .args([
            "--project",
            p,
            "--user-config",
            user_cfg.to_str().unwrap(),
            "init",
        ])
        .assert()
        .success();
    Command::cargo_bin("quay")
        .unwrap()
        .env("XDG_CONFIG_HOME", &cfg_home)
        .args([
            "--project",
            p,
            "--user-config",
            user_cfg.to_str().unwrap(),
            "remote",
            "add",
            "h",
            bare_url,
            "--default",
        ])
        .assert()
        .success();
    Command::cargo_bin("quay")
        .unwrap()
        .env("XDG_CONFIG_HOME", &cfg_home)
        .args([
            "--project",
            p,
            "--user-config",
            user_cfg.to_str().unwrap(),
            "add",
            "csv-parse",
        ])
        .assert()
        .success();
    Command::cargo_bin("quay")
        .unwrap()
        .env("XDG_CONFIG_HOME", &cfg_home)
        .args([
            "--project",
            p,
            "--user-config",
            user_cfg.to_str().unwrap(),
            "remove",
            "csv-parse",
        ])
        .assert()
        .success();
    Command::cargo_bin("quay")
        .unwrap()
        .env("XDG_CONFIG_HOME", &cfg_home)
        .args([
            "--project",
            p,
            "--user-config",
            user_cfg.to_str().unwrap(),
            "list",
        ])
        .assert()
        .success()
        .stdout(predicates::str::contains("(no local skills found)"));

    dir.child(".agents/skills/csv-parse")
        .assert(predicates::path::missing());
}

#[test]
fn quay_info_shows_skill_metadata() {
    let tmp = assert_fs::TempDir::new().unwrap();
    let bare = seed_hub(tmp.path());
    let (cfg_home, user_cfg) = isolated_user_config(tmp.path());

    let dir = tmp.child("project");
    std::fs::create_dir_all(dir.path()).unwrap();
    let p = dir.path().to_str().unwrap();
    let bare_url = bare.to_str().unwrap();

    Command::cargo_bin("quay")
        .unwrap()
        .env("XDG_CONFIG_HOME", &cfg_home)
        .args([
            "--project",
            p,
            "--user-config",
            user_cfg.to_str().unwrap(),
            "init",
        ])
        .assert()
        .success();
    Command::cargo_bin("quay")
        .unwrap()
        .env("XDG_CONFIG_HOME", &cfg_home)
        .args([
            "--project",
            p,
            "--user-config",
            user_cfg.to_str().unwrap(),
            "remote",
            "add",
            "h",
            bare_url,
            "--default",
        ])
        .assert()
        .success();
    Command::cargo_bin("quay")
        .unwrap()
        .env("XDG_CONFIG_HOME", &cfg_home)
        .args([
            "--project",
            p,
            "--user-config",
            user_cfg.to_str().unwrap(),
            "info",
            "csv-parse",
        ])
        .assert()
        .success()
        .stdout(predicates::str::contains("csv-parse"))
        .stdout(predicates::str::contains("1.0.0"))
        .stdout(predicates::str::contains("Parse CSV"));
}
