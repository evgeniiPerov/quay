//! Integration test for `quay add` collision semantics (Task 10).
//!
//! `quay add <skill>` blocks (exits non-zero with a helpful message) when
//! the skill already exists locally.
//!
//! `quay add --force <skill>` overwrites the existing local copy.
//!
//! Uses a real local bare-repo hub (quay fetches registries via `git
//! clone`, so a filesystem bare repo is the deterministic, offline way to
//! exercise this end-to-end) rather than mocking HTTP.

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

const REGISTRY: &str = r#"{
    "hub": "fixture",
    "generated_at": "2026-05-10T00:00:00Z",
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

/// Build a local bare-repo hub seeded with `registry.json` + the csv-parse
/// skill file, returning the bare repo path.
fn seed_hub(root: &Path) -> std::path::PathBuf {
    let bare = root.join("hub.git");
    git(
        root,
        &["init", "--bare", "-b", "main", bare.to_str().unwrap()],
    );
    let work = root.join("hub-work");
    git(
        root,
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

/// Set up a project directory pointing its default remote at `bare`, with
/// the csv-parse skill pre-installed locally with `local_content`.
fn seed_project(root: &Path, bare: &Path, local_content: &str) -> std::path::PathBuf {
    let proj = root.join("project");
    let quay_dir = proj.join(".quay");
    std::fs::create_dir_all(&quay_dir).unwrap();
    std::fs::write(
        quay_dir.join("config.toml"),
        format!(
            "[remotes.hub]\nurl = '{}'\ndefault = true\n",
            bare.to_str().unwrap()
        ),
    )
    .unwrap();
    let skill_dir = proj.join(".agents/skills/csv-parse");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(skill_dir.join("SKILL.md"), local_content).unwrap();
    proj
}

/// Isolate the host user config so `~/.config/quay/config.toml` never bleeds
/// in. Returns `(cfg_home, user_config_path)`.
fn isolated_user_config(root: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
    let cfg_home = root.join("cfg");
    let user_cfg = cfg_home.join("quay/config.toml");
    std::fs::create_dir_all(user_cfg.parent().unwrap()).unwrap();
    std::fs::write(&user_cfg, "").unwrap();
    (cfg_home, user_cfg)
}

/// `quay add <skill>` errors when the skill already exists and `--force` is absent.
#[test]
fn quay_add_blocks_when_skill_already_exists() {
    let tmp = assert_fs::TempDir::new().unwrap();

    let bare = seed_hub(tmp.path());
    let proj = seed_project(tmp.path(), &bare, "existing content");
    let (cfg_home, user_cfg) = isolated_user_config(tmp.path());

    let output = Command::cargo_bin("quay")
        .unwrap()
        .env("XDG_CONFIG_HOME", &cfg_home)
        .args([
            "--project",
            proj.to_str().unwrap(),
            "--user-config",
            user_cfg.to_str().unwrap(),
            "add",
            "csv-parse",
        ])
        .output()
        .unwrap();

    assert!(
        !output.status.success(),
        "expected non-zero exit when skill already exists"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("already exists") || stderr.contains("force"),
        "expected hint to use --force, got: {stderr}"
    );

    // The existing file should be unchanged.
    let content = std::fs::read_to_string(proj.join(".agents/skills/csv-parse/SKILL.md")).unwrap();
    assert_eq!(
        content, "existing content",
        "existing file must not be overwritten without --force"
    );
}

/// `quay add --force <skill>` overwrites the existing local copy.
#[test]
fn quay_add_force_overwrites_existing_skill() {
    let tmp = assert_fs::TempDir::new().unwrap();

    let bare = seed_hub(tmp.path());
    let proj = seed_project(tmp.path(), &bare, "stale content");
    let (cfg_home, user_cfg) = isolated_user_config(tmp.path());

    let output = Command::cargo_bin("quay")
        .unwrap()
        .env("XDG_CONFIG_HOME", &cfg_home)
        .args([
            "--project",
            proj.to_str().unwrap(),
            "--user-config",
            user_cfg.to_str().unwrap(),
            "add",
            "--force",
            "csv-parse",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "expected success with --force; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let content = std::fs::read_to_string(proj.join(".agents/skills/csv-parse/SKILL.md")).unwrap();
    assert_eq!(
        content, SKILL_MD,
        "--force must overwrite the existing file with new content"
    );
}
