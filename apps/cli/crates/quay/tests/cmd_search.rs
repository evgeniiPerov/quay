//! `quay search` reads a hub's registry.json. Uses real local bare-repo
//! hub(s) (quay fetches registries via `git clone`, so a filesystem bare
//! repo is the deterministic, offline way to exercise this end-to-end).

use assert_cmd::Command;
use predicates::prelude::PredicateBooleanExt;
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
            "version": "1.2.0",
            "description": "Parse CSV files with auto-delimiter detection.",
            "tags": ["data", "parsing"],
            "category": "backend",
            "path": "skills/csv-parse",
            "sha": "abc",
            "files": ["SKILL.md"]
        },
        "json-clean": {
            "version": "0.5.0",
            "description": "Clean JSON content.",
            "tags": ["data"],
            "category": "backend",
            "path": "skills/json-clean",
            "sha": "def",
            "files": ["SKILL.md"]
        }
    }
}"#;

/// Creates a bare repo at `tmp/<name>.git` whose `main` branch has
/// `registry.json` set to `registry`.
fn seed_hub(tmp: &Path, name: &str, registry: &str) -> std::path::PathBuf {
    let bare = tmp.join(format!("{name}.git"));
    git(
        tmp,
        &["init", "--bare", "-b", "main", bare.to_str().unwrap()],
    );
    let work = tmp.join(format!("{name}-work"));
    git(
        tmp,
        &["clone", bare.to_str().unwrap(), work.to_str().unwrap()],
    );
    std::fs::write(work.join("registry.json"), registry).unwrap();
    git(&work, &["add", "-A"]);
    git(&work, &["commit", "-m", "seed registry"]);
    git(&work, &["push", "origin", "main"]);
    bare
}

/// Isolates user config so the host `~/.config/quay` never bleeds in.
/// Returns (XDG_CONFIG_HOME dir, user-config file path).
fn isolated_user_config(tmp: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
    let cfg_home = tmp.join("cfg");
    let user_cfg = cfg_home.join("quay/config.toml");
    std::fs::create_dir_all(user_cfg.parent().unwrap()).unwrap();
    std::fs::write(&user_cfg, "").unwrap();
    (cfg_home, user_cfg)
}

#[test]
fn search_filters_by_substring() {
    let tmp = assert_fs::TempDir::new().unwrap();
    let bare = seed_hub(tmp.path(), "hub", REGISTRY);

    let proj = tmp.path().join("project");
    let p = proj.to_str().unwrap();
    let (cfg_home, user_cfg) = isolated_user_config(tmp.path());

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
            "search",
            "csv",
        ])
        .assert()
        .success()
        .stdout(predicates::str::contains("csv-parse"))
        .stdout(
            predicates::str::contains("Parse CSV").or(predicates::str::contains("auto-delimiter")),
        )
        .stdout(predicates::str::contains("json-clean").not());
}

#[test]
fn search_with_no_remotes_errors() {
    let tmp = assert_fs::TempDir::new().unwrap();
    let proj = tmp.path().join("project");
    let p = proj.to_str().unwrap();
    let (cfg_home, user_cfg) = isolated_user_config(tmp.path());

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
            "search",
            "anything",
        ])
        .assert()
        .failure()
        .stderr(predicates::str::contains("no remotes configured"));
}

#[test]
fn search_json_output_is_valid() {
    let tmp = assert_fs::TempDir::new().unwrap();
    let bare = seed_hub(tmp.path(), "hub", REGISTRY);

    let proj = tmp.path().join("project");
    let p = proj.to_str().unwrap();
    let (cfg_home, user_cfg) = isolated_user_config(tmp.path());

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
    let output = Command::cargo_bin("quay")
        .unwrap()
        .env("XDG_CONFIG_HOME", &cfg_home)
        .args([
            "--project",
            p,
            "--user-config",
            user_cfg.to_str().unwrap(),
            "--json",
            "search",
            "",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    let arr = parsed.as_array().unwrap();
    assert_eq!(arr.len(), 2);
}

#[test]
fn search_across_multiple_remotes() {
    let tmp = assert_fs::TempDir::new().unwrap();

    const REGISTRY_A: &str = r#"{
        "hub": "hub-a",
        "generated_at": "2026-05-08T00:00:00Z",
        "schema_version": 1,
        "skills": {
            "csv-parse": {
                "version": "1.2.0",
                "description": "Parse CSV files with auto-delimiter detection.",
                "tags": ["data", "parsing"],
                "category": "backend",
                "path": "skills/csv-parse",
                "sha": "abc",
                "files": ["SKILL.md"]
            }
        }
    }"#;
    const REGISTRY_B: &str = r#"{
        "hub": "hub-b",
        "generated_at": "2026-05-08T00:00:00Z",
        "schema_version": 1,
        "skills": {
            "csv-writer": {
                "version": "2.0.0",
                "description": "Write CSV files.",
                "tags": ["data"],
                "category": "backend",
                "path": "skills/csv-writer",
                "sha": "ghi",
                "files": ["SKILL.md"]
            }
        }
    }"#;

    let bare_a = seed_hub(tmp.path(), "hub-a", REGISTRY_A);
    let bare_b = seed_hub(tmp.path(), "hub-b", REGISTRY_B);

    let proj = tmp.path().join("project");
    let p = proj.to_str().unwrap();
    let (cfg_home, user_cfg) = isolated_user_config(tmp.path());

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
            "a",
            bare_a.to_str().unwrap(),
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
            "remote",
            "add",
            "b",
            bare_b.to_str().unwrap(),
        ])
        .assert()
        .success();

    // Searching across all remotes finds hits from both hubs.
    Command::cargo_bin("quay")
        .unwrap()
        .env("XDG_CONFIG_HOME", &cfg_home)
        .args([
            "--project",
            p,
            "--user-config",
            user_cfg.to_str().unwrap(),
            "search",
            "csv",
        ])
        .assert()
        .success()
        .stdout(predicates::str::contains("csv-parse"))
        .stdout(predicates::str::contains("csv-writer"));

    // Scoping to a single remote only returns that remote's hits.
    Command::cargo_bin("quay")
        .unwrap()
        .env("XDG_CONFIG_HOME", &cfg_home)
        .args([
            "--project",
            p,
            "--user-config",
            user_cfg.to_str().unwrap(),
            "search",
            "csv",
            "--remote",
            "b",
        ])
        .assert()
        .success()
        .stdout(predicates::str::contains("csv-writer"))
        .stdout(predicates::str::contains("csv-parse").not());
}
