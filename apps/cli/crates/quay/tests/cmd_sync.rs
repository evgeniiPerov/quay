#![cfg(debug_assertions)]

use assert_cmd::Command;
use assert_fs::prelude::*;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

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
            "sha": "abc123",
            "files": ["SKILL.md"]
        }
    }
}"#;

const SKILL_MD: &str = "---\nname: csv-parse\ndescription: Parse CSV.\nversion: 1.0.0\n---\nbody\n";

#[tokio::test]
async fn sync_restores_deleted_file() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/foo/bar/main/registry.json"))
        .respond_with(ResponseTemplate::new(200).set_body_string(REGISTRY))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/foo/bar/main/skills/csv-parse/SKILL.md"))
        .respond_with(ResponseTemplate::new(200).set_body_string(SKILL_MD))
        .mount(&server)
        .await;
    // Sync uses fetch_file_at with the lockfile's recorded sha (the registry's `sha` field).
    Mock::given(method("GET"))
        .and(path("/foo/bar/abc123/skills/csv-parse/SKILL.md"))
        .respond_with(ResponseTemplate::new(200).set_body_string(SKILL_MD))
        .mount(&server)
        .await;

    let dir = assert_fs::TempDir::new().unwrap();
    let p = dir.path().to_str().unwrap().to_string();
    let url = "https://github.com/foo/bar.git".to_string();
    let base = server.uri();
    let p_install = p.clone();
    let url_install = url.clone();
    let base_install = base.clone();
    tokio::task::spawn_blocking(move || {
        Command::cargo_bin("quay")
            .unwrap()
            .args(["--project", &p_install, "init"])
            .assert()
            .success();
        Command::cargo_bin("quay")
            .unwrap()
            .args([
                "--project",
                &p_install,
                "remote",
                "add",
                "h",
                &url_install,
                "--default",
            ])
            .assert()
            .success();
        Command::cargo_bin("quay")
            .unwrap()
            .env("QUAY_GITHUB_BASE_URL", &base_install)
            .args(["--project", &p_install, "add", "csv-parse"])
            .assert()
            .success();
    })
    .await
    .unwrap();

    let installed = dir.child(".agents/skills/csv-parse/SKILL.md");
    installed.assert(predicates::path::exists());
    std::fs::remove_file(installed.path()).unwrap();
    installed.assert(predicates::path::missing());

    let p_sync = p.clone();
    let base_sync = base.clone();
    tokio::task::spawn_blocking(move || {
        Command::cargo_bin("quay")
            .unwrap()
            .env("QUAY_GITHUB_BASE_URL", &base_sync)
            .args(["--project", &p_sync, "sync"])
            .assert()
            .success()
            .stdout(predicates::str::contains("refetched csv-parse"));
    })
    .await
    .unwrap();
    installed.assert(predicates::path::exists());
}
