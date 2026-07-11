//! `quay update` pulls the latest version of an installed skill from the
//! remote registry. Uses a real local bare-repo hub (quay fetches registries
//! via `git clone`, so a filesystem bare repo is the deterministic, offline
//! way to exercise this end-to-end): seed the hub with v1, install, push v2
//! to the same hub, then update.

use assert_cmd::Command;
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

const REG_OLD: &str = r#"{
    "hub": "fixture",
    "generated_at": "2026-05-08T00:00:00Z",
    "schema_version": 1,
    "skills": {
        "csv-parse": {
            "version": "1.0.0",
            "description": "Parse CSV.",
            "tags": [],
            "path": "skills/csv-parse",
            "sha": "old",
            "files": ["SKILL.md"]
        }
    }
}"#;

const REG_NEW: &str = r#"{
    "hub": "fixture",
    "generated_at": "2026-05-08T00:00:00Z",
    "schema_version": 1,
    "skills": {
        "csv-parse": {
            "version": "1.2.0",
            "description": "Parse CSV.",
            "tags": [],
            "path": "skills/csv-parse",
            "sha": "new",
            "files": ["SKILL.md"]
        }
    }
}"#;

const SKILL_OLD: &str =
    "---\nname: csv-parse\ndescription: Parse CSV.\nversion: 1.0.0\n---\nold body\n";
const SKILL_NEW: &str =
    "---\nname: csv-parse\ndescription: Parse CSV.\nversion: 1.2.0\n---\nnew body\n";

/// Set up a bare-repo hub at `<tmp>/hub.git`, seeded with `registry` +
/// `skill_md` under `skills/csv-parse/SKILL.md`, committed and pushed to
/// `main`. Returns the bare repo path.
fn seed_hub(tmp: &Path, registry: &str, skill_md: &str) -> std::path::PathBuf {
    let bare = tmp.join("hub.git");
    if !bare.exists() {
        git(
            tmp,
            &["init", "--bare", "-b", "main", bare.to_str().unwrap()],
        );
    }
    let work = tmp.join("hub-work");
    if work.exists() {
        std::fs::remove_dir_all(&work).unwrap();
    }
    git(
        tmp,
        &["clone", bare.to_str().unwrap(), work.to_str().unwrap()],
    );
    std::fs::write(work.join("registry.json"), registry).unwrap();
    let skill_dir = work.join("skills/csv-parse");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(skill_dir.join("SKILL.md"), skill_md).unwrap();
    git(&work, &["add", "-A"]);
    git(&work, &["commit", "-m", "seed registry"]);
    git(&work, &["push", "origin", "main"]);
    bare
}

/// Isolate the host user config so `~/.config/quay` never bleeds into the
/// test. Returns `(XDG_CONFIG_HOME dir, empty user-config file path)`.
fn isolated_user_config(tmp: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
    let cfg_home = tmp.join("cfg");
    let user_cfg = cfg_home.join("quay/config.toml");
    std::fs::create_dir_all(user_cfg.parent().unwrap()).unwrap();
    std::fs::write(&user_cfg, "").unwrap();
    (cfg_home, user_cfg)
}

#[test]
fn update_pulls_newer_version() {
    let tmp = assert_fs::TempDir::new().unwrap();
    let (cfg_home, user_cfg) = isolated_user_config(tmp.path());

    // 1. Seed the hub with v1.0.0 and install it into a fresh project.
    let bare = seed_hub(tmp.path(), REG_OLD, SKILL_OLD);
    let proj = tmp.path().join("project");
    let p = proj.to_str().unwrap();

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
            bare.to_str().unwrap(),
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

    // 2. Push v1.2.0 to the same hub.
    seed_hub(tmp.path(), REG_NEW, SKILL_NEW);

    // 3. `quay update csv-parse` should pull the newer version.
    Command::cargo_bin("quay")
        .unwrap()
        .env("XDG_CONFIG_HOME", &cfg_home)
        .args([
            "--project",
            p,
            "--user-config",
            user_cfg.to_str().unwrap(),
            "update",
            "csv-parse",
        ])
        .assert()
        .success();

    let body = std::fs::read_to_string(proj.join(".agents/skills/csv-parse/SKILL.md")).unwrap();
    assert!(body.contains("1.2.0"));
    assert!(body.contains("new body"));
}

#[test]
fn update_dry_run_does_not_write() {
    let tmp = assert_fs::TempDir::new().unwrap();
    let (cfg_home, user_cfg) = isolated_user_config(tmp.path());

    // 1. Seed the hub with v1.0.0 and install it into a fresh project.
    let bare = seed_hub(tmp.path(), REG_OLD, SKILL_OLD);
    let proj = tmp.path().join("project");
    let p = proj.to_str().unwrap();

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
            bare.to_str().unwrap(),
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

    // 2. Push v1.2.0 to the same hub.
    seed_hub(tmp.path(), REG_NEW, SKILL_NEW);

    // 3. `quay update --dry-run` should report the pending update without writing it.
    Command::cargo_bin("quay")
        .unwrap()
        .env("XDG_CONFIG_HOME", &cfg_home)
        .args([
            "--project",
            p,
            "--user-config",
            user_cfg.to_str().unwrap(),
            "update",
            "--dry-run",
        ])
        .assert()
        .success()
        .stdout(predicates::str::contains("would update csv-parse"));

    let body = std::fs::read_to_string(proj.join(".agents/skills/csv-parse/SKILL.md")).unwrap();
    assert!(body.contains("1.0.0"));
    assert!(body.contains("old body"));
}
