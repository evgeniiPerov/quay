//! `quay outdated` flags a hand-written (non-frontmatter) skill by its folder
//! content hash. Uses a real local bare-repo hub (quay fetches registries via
//! `git clone`, so a filesystem bare repo is the deterministic, offline way to
//! exercise this end-to-end). The installed skill is written directly on disk.

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

// Registry advertising a hand-written skill. content_hash is deliberately
// all-zeros — it cannot equal the installed folder's real sha256, so `outdated`
// must flag the skill via the content-hash branch.
const REGISTRY_STALE_HASH: &str = r#"{
    "hub": "fixture",
    "generated_at": "2026-05-08T00:00:00Z",
    "schema_version": 1,
    "skills": {
        "csv-parse": {
            "version": "0.0.0",
            "description": "Parse CSV.",
            "tags": [],
            "path": "skills/csv-parse",
            "sha": "abc",
            "files": ["SKILL.md"],
            "source_format": "slash_command",
            "content_hash": "0000000000000000000000000000000000000000000000000000000000000000"
        }
    }
}"#;

// SlashCommand skill: first non-blank line is `# /<name>`, no YAML frontmatter.
const SKILL_MD: &str = "# /csv-parse\n\nParse a CSV file.\n";

#[test]
fn outdated_flags_hand_written_skill_by_content_hash() {
    let tmp = assert_fs::TempDir::new().unwrap();

    // 1. Build a local bare-repo hub whose registry.json advertises csv-parse
    //    with a stale (all-zeros) content_hash.
    let bare = tmp.path().join("hub.git");
    git(tmp.path(), &["init", "--bare", "-b", "main", bare.to_str().unwrap()]);
    let work = tmp.path().join("hub-work");
    git(tmp.path(), &["clone", bare.to_str().unwrap(), work.to_str().unwrap()]);
    std::fs::write(work.join("registry.json"), REGISTRY_STALE_HASH).unwrap();
    git(&work, &["add", "-A"]);
    git(&work, &["commit", "-m", "seed registry"]);
    git(&work, &["push", "origin", "main"]);

    // 2. Project with the hand-written skill installed on disk + a remote
    //    pointing at the bare hub.
    let proj = tmp.path().join("project");
    let quay_dir = proj.join(".quay");
    std::fs::create_dir_all(&quay_dir).unwrap();
    std::fs::write(
        quay_dir.join("config.toml"),
        format!(
            "[remotes.hub]\nurl = \"{}\"\ndefault = true\n",
            bare.to_str().unwrap()
        ),
    )
    .unwrap();
    let skill_dir = proj.join(".agents/skills/csv-parse");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(skill_dir.join("SKILL.md"), SKILL_MD).unwrap();

    // 3. Isolate user config so the host ~/.config/quay never bleeds in.
    let cfg_home = tmp.path().join("cfg");
    let user_cfg = cfg_home.join("quay/config.toml");
    std::fs::create_dir_all(user_cfg.parent().unwrap()).unwrap();
    std::fs::write(&user_cfg, "").unwrap();

    // 4. outdated must list csv-parse as stale (local folder hash != zeros).
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
        .stdout(predicates::str::contains("csv-parse"));
}
