//! `quay diff <skill>` — read-only local-vs-harbor report. Real bare-repo hub
//! so the clone, listing and history walk all run for real.

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

const SKILL_V1: &str = "---\nname: csv-parse\ndescription: Parse CSV.\nversion: 1.0.0\n---\nbody\n";

fn registry(version: &str) -> String {
    format!(
        r#"{{"hub":"fixture","generated_at":"2026-05-08T00:00:00Z","schema_version":1,
        "skills":{{"csv-parse":{{"version":"{version}","description":"Parse CSV.","tags":[],
        "path":"skills/csv-parse","sha":"abc","files":["SKILL.md"],
        "source_format":"frontmatter"}}}}}}"#
    )
}

/// Hub whose `skills/csv-parse` ends at `hub_files`, having passed through
/// `SKILL_V1` first, plus a project holding `local_files`.
fn fixture(
    tmp: &Path,
    hub_files: &[(&str, &str)],
    local_files: &[(&str, &str)],
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

    // First commit: the state the project installed from.
    let skill = work.join("skills/csv-parse");
    std::fs::create_dir_all(&skill).unwrap();
    std::fs::write(work.join("registry.json"), registry("1.0.0")).unwrap();
    std::fs::write(skill.join("SKILL.md"), SKILL_V1).unwrap();
    git(&work, &["add", "-A"]);
    git(&work, &["commit", "-m", "seed"]);

    // Second commit: where the hub is now.
    std::fs::remove_dir_all(&skill).unwrap();
    for (rel, body) in hub_files {
        let full = skill.join(rel);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(full, body).unwrap();
    }
    git(&work, &["add", "-A"]);
    git(&work, &["commit", "--allow-empty", "-m", "hub moves on"]);
    git(&work, &["push", "origin", "main"]);

    let proj = tmp.join("project");
    std::fs::create_dir_all(proj.join(".quay")).unwrap();
    std::fs::write(
        proj.join(".quay/config.toml"),
        format!(
            "[remotes.hub]\nurl = '{}'\ndefault = true\n",
            bare.to_str().unwrap()
        ),
    )
    .unwrap();
    for (rel, body) in local_files {
        let full = proj.join(".agents/skills/csv-parse").join(rel);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(full, body).unwrap();
    }

    let cfg_home = tmp.join("cfg");
    let user_cfg = cfg_home.join("quay/config.toml");
    std::fs::create_dir_all(user_cfg.parent().unwrap()).unwrap();
    std::fs::write(&user_cfg, "").unwrap();

    (proj, user_cfg, cfg_home)
}

fn diff(proj: &Path, user_cfg: &Path, cfg_home: &Path, extra: &[&str]) -> Command {
    let mut c = Command::cargo_bin("quay").unwrap();
    c.env("XDG_CONFIG_HOME", cfg_home).args([
        "--project",
        proj.to_str().unwrap(),
        "--user-config",
        user_cfg.to_str().unwrap(),
        "diff",
        "csv-parse",
    ]);
    c.args(extra);
    c
}

#[test]
fn reports_a_sibling_file_the_hub_changed() {
    let tmp = assert_fs::TempDir::new().unwrap();
    let (proj, user_cfg, cfg_home) = fixture(
        tmp.path(),
        &[("SKILL.md", SKILL_V1), ("references/api.md", "GET /v2\n")],
        &[("SKILL.md", SKILL_V1), ("references/api.md", "GET /v1\n")],
    );

    diff(&proj, &user_cfg, &cfg_home, &[])
        .assert()
        .success()
        .stdout(predicates::str::contains("references/api.md"))
        // Pull-oriented: `+` is what the hub would give you.
        .stdout(predicates::str::contains("+GET /v2"))
        .stdout(predicates::str::contains("-GET /v1"));
}

#[test]
fn says_up_to_date_without_printing_a_diff() {
    let tmp = assert_fs::TempDir::new().unwrap();
    let (proj, user_cfg, cfg_home) = fixture(
        tmp.path(),
        &[("SKILL.md", SKILL_V1)],
        &[("SKILL.md", SKILL_V1)],
    );

    diff(&proj, &user_cfg, &cfg_home, &[])
        .assert()
        .success()
        .stdout(predicates::str::contains("up to date"));
}

#[test]
fn json_carries_the_verdict_and_per_file_kinds() {
    let tmp = assert_fs::TempDir::new().unwrap();
    let (proj, user_cfg, cfg_home) = fixture(
        tmp.path(),
        &[("SKILL.md", SKILL_V1), ("scripts/new.sh", "echo hi\n")],
        &[("SKILL.md", SKILL_V1)],
    );

    let out = diff(&proj, &user_cfg, &cfg_home, &["--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).expect("valid JSON");

    assert_eq!(v["skill"], "csv-parse");
    assert_eq!(v["verdict"], "hub_newer");
    let files = v["files"].as_array().unwrap();
    let new_sh = files
        .iter()
        .find(|f| f["path"] == "scripts/new.sh")
        .expect("the added file is listed");
    assert_eq!(new_sh["change"], "only_on_hub");
}

#[test]
fn an_unknown_skill_is_an_error_not_an_empty_report() {
    let tmp = assert_fs::TempDir::new().unwrap();
    let (proj, user_cfg, cfg_home) = fixture(
        tmp.path(),
        &[("SKILL.md", SKILL_V1)],
        &[("SKILL.md", SKILL_V1)],
    );

    Command::cargo_bin("quay")
        .unwrap()
        .env("XDG_CONFIG_HOME", &cfg_home)
        .args([
            "--project",
            proj.to_str().unwrap(),
            "--user-config",
            user_cfg.to_str().unwrap(),
            "diff",
            "not-installed",
        ])
        .assert()
        .failure()
        .stderr(predicates::str::contains("not-installed"));
}
