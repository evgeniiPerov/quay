//! `quay outdated` end-to-end against a real local bare-repo hub (no network,
//! deterministic). quay fetches registries via `git clone` (see
//! `quay-core::CloneFetcher`), so a filesystem bare repo is the offline way to
//! exercise this: HTTP mocking never intercepts a `git clone`.

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

const REGISTRY_OLD: &str = r#"{
    "hub": "fixture",
    "generated_at": "2026-05-08T00:00:00Z",
    "schema_version": 1,
    "skills": {
        "csv-parse": {
            "version": "1.0.0",
            "description": "Parse CSV.",
            "tags": [],
            "path": "skills/csv-parse",
            "sha": "abc",
            "files": ["SKILL.md"]
        }
    }
}"#;

const REGISTRY_NEW: &str = r#"{
    "hub": "fixture",
    "generated_at": "2026-05-08T00:00:00Z",
    "schema_version": 1,
    "skills": {
        "csv-parse": {
            "version": "1.2.0",
            "description": "Parse CSV.",
            "tags": [],
            "path": "skills/csv-parse",
            "sha": "abc",
            "files": ["SKILL.md"]
        }
    }
}"#;

const SKILL_MD: &str = "---\nname: csv-parse\ndescription: Parse CSV.\nversion: 1.0.0\n---\nbody\n";

#[test]
fn outdated_lists_stale_skill() {
    let tmp = assert_fs::TempDir::new().unwrap();

    // 1. Build a local bare-repo hub whose registry.json advertises
    //    csv-parse@1.0.0.
    let bare = tmp.path().join("hub.git");
    git(
        tmp.path(),
        &["init", "--bare", "-b", "main", bare.to_str().unwrap()],
    );
    let work = tmp.path().join("hub-work");
    git(
        tmp.path(),
        &["clone", bare.to_str().unwrap(), work.to_str().unwrap()],
    );
    std::fs::write(work.join("registry.json"), REGISTRY_OLD).unwrap();
    let skill_dir = work.join("skills/csv-parse");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(skill_dir.join("SKILL.md"), SKILL_MD).unwrap();
    git(&work, &["add", "-A"]);
    git(&work, &["commit", "-m", "seed registry"]);
    git(&work, &["push", "origin", "main"]);

    // 2. Project pointed at the bare hub.
    let proj = tmp.path().join("project");
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

    // 3. Isolate user config so the host ~/.config/quay never bleeds in.
    let cfg_home = tmp.path().join("cfg");
    let user_cfg = cfg_home.join("quay/config.toml");
    std::fs::create_dir_all(user_cfg.parent().unwrap()).unwrap();
    std::fs::write(&user_cfg, "").unwrap();

    // 4. Install csv-parse@1.0.0 from the hub.
    Command::cargo_bin("quay")
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
        .assert()
        .success();

    let installed =
        std::fs::read_to_string(proj.join(".agents/skills/csv-parse/SKILL.md")).unwrap();
    assert!(
        installed.contains("version: 1.0.0"),
        "installed skill must carry version 1.0.0 frontmatter; got {installed:?}"
    );

    // 5. Bump the hub's registry.json to 1.2.0 (second commit, same bare hub).
    std::fs::write(work.join("registry.json"), REGISTRY_NEW).unwrap();
    git(&work, &["add", "-A"]);
    git(&work, &["commit", "-m", "bump to 1.2.0"]);
    git(&work, &["push", "origin", "main"]);

    // 6. outdated must list csv-parse as stale, showing the new available version.
    Command::cargo_bin("quay")
        .unwrap()
        .env("XDG_CONFIG_HOME", &cfg_home)
        .args([
            "--project",
            proj.to_str().unwrap(),
            "--user-config",
            user_cfg.to_str().unwrap(),
            "outdated",
        ])
        .assert()
        .success()
        .stdout(predicates::str::contains("csv-parse"))
        .stdout(predicates::str::contains("local -> 1.2.0"));
}

#[test]
fn outdated_when_no_install_says_no_skills() {
    let tmp = assert_fs::TempDir::new().unwrap();
    let proj = tmp.path().join("project");

    // Isolate user config so the host ~/.config/quay never bleeds in a warning.
    let cfg_home = tmp.path().join("cfg");
    let user_cfg = cfg_home.join("quay/config.toml");
    std::fs::create_dir_all(user_cfg.parent().unwrap()).unwrap();
    std::fs::write(&user_cfg, "").unwrap();

    Command::cargo_bin("quay")
        .unwrap()
        .env("XDG_CONFIG_HOME", &cfg_home)
        .args([
            "--project",
            proj.to_str().unwrap(),
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
            proj.to_str().unwrap(),
            "--user-config",
            user_cfg.to_str().unwrap(),
            "outdated",
        ])
        .assert()
        .success()
        .stdout(predicates::str::contains("(everything up to date)"));
}
