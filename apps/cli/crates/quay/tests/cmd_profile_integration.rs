//! End-to-end test: `quay search` uses the active profile's remotes and
//! `--profile` on the root command overrides the active profile for one call.
//!
//! Uses local bare-repo hubs (quay fetches registries via `git clone`, so a
//! filesystem bare repo is the deterministic, offline way to exercise this
//! end-to-end — no network access, no HTTP mocking).

use assert_cmd::Command;
use assert_fs::prelude::*;
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

fn registry_json(skill_name: &str, description: &str) -> String {
    format!(
        r#"{{
    "hub": "fixture",
    "generated_at": "2026-05-08T00:00:00Z",
    "schema_version": 1,
    "skills": {{
        "{skill_name}": {{
            "version": "0.1.0",
            "description": "{description}",
            "tags": [],
            "path": "skills/{skill_name}",
            "sha": "abc",
            "files": ["SKILL.md"]
        }}
    }}
}}"#
    )
}

/// Build a local bare-repo hub at `<parent>/<name>.git` whose `registry.json`
/// advertises a single skill, seeded on `main`.
fn init_hub(parent: &Path, name: &str, skill_name: &str, description: &str) -> std::path::PathBuf {
    let bare = parent.join(format!("{name}.git"));
    git(
        parent,
        &["init", "--bare", "-b", "main", bare.to_str().unwrap()],
    );
    let work = parent.join(format!("{name}-work"));
    git(
        parent,
        &["clone", bare.to_str().unwrap(), work.to_str().unwrap()],
    );
    std::fs::write(
        work.join("registry.json"),
        registry_json(skill_name, description),
    )
    .unwrap();
    git(&work, &["add", "-A"]);
    git(&work, &["commit", "-m", "seed registry"]);
    git(&work, &["push", "origin", "main"]);
    bare
}

#[test]
fn search_uses_active_profile_remotes() {
    let tmp = assert_fs::TempDir::new().unwrap();

    // hub-a → skill-alpha (work profile's hub)
    let bare_a = init_hub(tmp.path(), "hub-a", "skill-alpha", "Alpha test skill.");
    // hub-b → skill-bravo (personal profile's hub)
    let bare_b = init_hub(tmp.path(), "hub-b", "skill-bravo", "Bravo test skill.");

    let user = tmp.child("user.toml");
    std::fs::write(user.path(), "").unwrap();
    let project = tmp.child("project");
    std::fs::create_dir_all(project.path()).unwrap();

    let p = project.path().to_str().unwrap().to_string();
    let u = user.path().to_str().unwrap().to_string();
    let url_a = bare_a.to_str().unwrap().to_string();
    let url_b = bare_b.to_str().unwrap().to_string();

    // Isolate user config so the host ~/.config/quay never bleeds in.
    let cfg_home = tmp.path().join("cfg");
    std::fs::create_dir_all(&cfg_home).unwrap();

    // init project so .quay/ exists
    Command::cargo_bin("quay")
        .unwrap()
        .env("XDG_CONFIG_HOME", &cfg_home)
        .args(["--user-config", &u, "--project", &p, "init"])
        .assert()
        .success();

    // Add "work" profile with hub-a (skill-alpha); first add → automatically active.
    Command::cargo_bin("quay")
        .unwrap()
        .env("XDG_CONFIG_HOME", &cfg_home)
        .args([
            "--user-config",
            &u,
            "--project",
            &p,
            "profile",
            "add",
            "work",
            "--email",
            "e@work",
            "--remote",
            &format!("h={}", url_a),
        ])
        .assert()
        .success();

    // Add "personal" profile with hub-b (skill-bravo).
    Command::cargo_bin("quay")
        .unwrap()
        .env("XDG_CONFIG_HOME", &cfg_home)
        .args([
            "--user-config",
            &u,
            "--project",
            &p,
            "profile",
            "add",
            "personal",
            "--email",
            "e@home",
            "--remote",
            &format!("h={}", url_b),
        ])
        .assert()
        .success();

    // Explicitly switch active to "work".
    Command::cargo_bin("quay")
        .unwrap()
        .env("XDG_CONFIG_HOME", &cfg_home)
        .args([
            "--user-config",
            &u,
            "--project",
            &p,
            "profile",
            "use",
            "work",
        ])
        .assert()
        .success();

    // Default search (active = work) → sees skill-alpha only.
    Command::cargo_bin("quay")
        .unwrap()
        .env("XDG_CONFIG_HOME", &cfg_home)
        .args(["--user-config", &u, "--project", &p, "search", "skill"])
        .assert()
        .success()
        .stdout(predicates::str::contains("skill-alpha"))
        .stdout(predicates::str::contains("skill-bravo").not());

    // --profile=personal flips to hub-b without changing the active profile.
    Command::cargo_bin("quay")
        .unwrap()
        .env("XDG_CONFIG_HOME", &cfg_home)
        .args([
            "--user-config",
            &u,
            "--project",
            &p,
            "--profile",
            "personal",
            "search",
            "skill",
        ])
        .assert()
        .success()
        .stdout(predicates::str::contains("skill-bravo"))
        .stdout(predicates::str::contains("skill-alpha").not());

    // Verify active profile is still "work" (profile use was not changed).
    Command::cargo_bin("quay")
        .unwrap()
        .env("XDG_CONFIG_HOME", &cfg_home)
        .args(["--user-config", &u, "--project", &p, "profile", "current"])
        .assert()
        .success()
        .stdout(predicates::str::starts_with("work"));
}
