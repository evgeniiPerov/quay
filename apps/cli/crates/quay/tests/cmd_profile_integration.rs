// End-to-end test: `quay search` uses the active profile's remotes and
// `--profile` on the root command overrides the active profile for one call.
//
// This file uses QUAY_GITHUB_BASE_URL (debug-builds only) to redirect all HTTP
// fetches to a wiremock instance, so no real network access is needed.
#![cfg(debug_assertions)]

use assert_cmd::Command;
use assert_fs::prelude::*;
use predicates::prelude::PredicateBooleanExt;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

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

#[tokio::test]
async fn search_uses_active_profile_remotes() {
    // Single wiremock server serves two distinct "GitHub" repo paths.
    let server = MockServer::start().await;

    // org-a/hub-a → skill-alpha (work profile's hub)
    Mock::given(method("GET"))
        .and(path("/org-a/hub-a/main/registry.json"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(registry_json("skill-alpha", "Alpha test skill.")),
        )
        .mount(&server)
        .await;

    // org-b/hub-b → skill-bravo (personal profile's hub)
    Mock::given(method("GET"))
        .and(path("/org-b/hub-b/main/registry.json"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(registry_json("skill-bravo", "Bravo test skill.")),
        )
        .mount(&server)
        .await;

    let tmp = assert_fs::TempDir::new().unwrap();
    let user = tmp.child("user.toml");
    std::fs::write(user.path(), "").unwrap();
    let project = tmp.child("project");
    std::fs::create_dir_all(project.path()).unwrap();

    let p = project.path().to_str().unwrap().to_string();
    let u = user.path().to_str().unwrap().to_string();
    let base = server.uri();

    // GitHub-style URLs: parse_owner_repo extracts org-a/hub-a and org-b/hub-b.
    let url_a = "https://github.com/org-a/hub-a.git".to_string();
    let url_b = "https://github.com/org-b/hub-b.git".to_string();

    tokio::task::spawn_blocking(move || {
        // init project so .quay/ exists
        Command::cargo_bin("quay")
            .unwrap()
            .args(["--user-config", &u, "--project", &p, "init"])
            .assert()
            .success();

        // Add "work" profile with hub-a (skill-alpha); first add → automatically active.
        Command::cargo_bin("quay")
            .unwrap()
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
            .env("QUAY_GITHUB_BASE_URL", &base)
            .args(["--user-config", &u, "--project", &p, "search", "skill"])
            .assert()
            .success()
            .stdout(predicates::str::contains("skill-alpha"))
            .stdout(predicates::str::contains("skill-bravo").not());

        // --profile=personal flips to hub-b without changing the active profile.
        Command::cargo_bin("quay")
            .unwrap()
            .env("QUAY_GITHUB_BASE_URL", &base)
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
            .args(["--user-config", &u, "--project", &p, "profile", "current"])
            .assert()
            .success()
            .stdout(predicates::str::starts_with("work"));
    })
    .await
    .unwrap();
}
