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

// Same skill as a frontmatter skill at version 1.0.0, advertised by the hub at
// the *same* version but with a content_hash that cannot match the installed
// bytes. This is the shape of a hub edit that shipped without a version bump.
const REGISTRY_SAME_VERSION_STALE_HASH: &str = r#"{
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
            "files": ["SKILL.md"],
            "source_format": "frontmatter",
            "content_hash": "0000000000000000000000000000000000000000000000000000000000000000"
        }
    }
}"#;

const FRONTMATTER_SKILL_MD: &str =
    "---\nname: csv-parse\ndescription: Parse CSV.\nversion: 1.0.0\n---\nbody\n";

/// Bare-repo hub serving `registry`, plus a project with `skill_md` installed
/// and an isolated user config. Returns (project dir, user config path,
/// XDG_CONFIG_HOME).
fn fixture(
    tmp: &Path,
    registry: &str,
    skill_md: &str,
) -> (std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
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
    std::fs::write(work.join("registry.json"), registry).unwrap();
    git(&work, &["add", "-A"]);
    git(&work, &["commit", "-m", "seed registry"]);
    git(&work, &["push", "origin", "main"]);

    let proj = tmp.join("project");
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
    std::fs::write(skill_dir.join("SKILL.md"), skill_md).unwrap();

    // Isolate user config so the host ~/.config/quay never bleeds in.
    let cfg_home = tmp.join("cfg");
    let user_cfg = cfg_home.join("quay/config.toml");
    std::fs::create_dir_all(user_cfg.parent().unwrap()).unwrap();
    std::fs::write(&user_cfg, "").unwrap();

    (proj, user_cfg, cfg_home)
}

fn outdated(proj: &Path, user_cfg: &Path, cfg_home: &Path) -> assert_cmd::assert::Assert {
    Command::cargo_bin("quay")
        .unwrap()
        .env("XDG_CONFIG_HOME", cfg_home)
        .args([
            "--project",
            proj.to_str().unwrap(),
            "--user-config",
            user_cfg.to_str().unwrap(),
            "outdated",
        ])
        .assert()
        .success()
}

#[test]
fn outdated_flags_hand_written_skill_by_content_hash() {
    let tmp = assert_fs::TempDir::new().unwrap();
    let (proj, user_cfg, cfg_home) = fixture(tmp.path(), REGISTRY_STALE_HASH, SKILL_MD);
    // Local folder hash != zeros, so csv-parse must be listed as stale.
    outdated(&proj, &user_cfg, &cfg_home).stdout(predicates::str::contains("csv-parse"));
}

#[test]
fn outdated_flags_frontmatter_skill_edited_on_hub_without_a_version_bump() {
    let tmp = assert_fs::TempDir::new().unwrap();
    let (proj, user_cfg, cfg_home) = fixture(
        tmp.path(),
        REGISTRY_SAME_VERSION_STALE_HASH,
        FRONTMATTER_SKILL_MD,
    );
    // Both sides say 1.0.0, so semver alone reports nothing. The content hashes
    // differ, and that has to reach the user.
    outdated(&proj, &user_cfg, &cfg_home)
        .stdout(predicates::str::contains("csv-parse"))
        .stdout(predicates::str::contains("differs from hub at 1.0.0"))
        // `quay update` acts on semver upgrades only, so the row has to say
        // what actually resolves it.
        .stdout(predicates::str::contains("quay add <name> --force"));
}
