#![cfg(debug_assertions)]

use assert_cmd::Command;
use assert_fs::prelude::*;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

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

#[tokio::test]
async fn update_pulls_newer_version() {
    let server_install = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/foo/bar/main/registry.json"))
        .respond_with(ResponseTemplate::new(200).set_body_string(REG_OLD))
        .mount(&server_install)
        .await;
    Mock::given(method("GET"))
        .and(path("/foo/bar/main/skills/csv-parse/SKILL.md"))
        .respond_with(ResponseTemplate::new(200).set_body_string(SKILL_OLD))
        .mount(&server_install)
        .await;

    let dir = assert_fs::TempDir::new().unwrap();
    let p = dir.path().to_str().unwrap().to_string();
    let url = "https://github.com/foo/bar.git".to_string();
    let install_base = server_install.uri();
    let p_install = p.clone();
    let url_install = url.clone();
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
            .env("QUAY_GITHUB_BASE_URL", &install_base)
            .args(["--project", &p_install, "add", "csv-parse"])
            .assert()
            .success();
    })
    .await
    .unwrap();

    let server_new = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/foo/bar/main/registry.json"))
        .respond_with(ResponseTemplate::new(200).set_body_string(REG_NEW))
        .mount(&server_new)
        .await;
    Mock::given(method("GET"))
        .and(path("/foo/bar/main/skills/csv-parse/SKILL.md"))
        .respond_with(ResponseTemplate::new(200).set_body_string(SKILL_NEW))
        .mount(&server_new)
        .await;

    let new_base = server_new.uri();
    let p_update = p.clone();
    tokio::task::spawn_blocking(move || {
        Command::cargo_bin("quay")
            .unwrap()
            .env("QUAY_GITHUB_BASE_URL", &new_base)
            .args(["--project", &p_update, "update", "csv-parse"])
            .assert()
            .success();
    })
    .await
    .unwrap();

    let body =
        std::fs::read_to_string(dir.child(".agents/skills/csv-parse/SKILL.md").path()).unwrap();
    assert!(body.contains("1.2.0"));
    assert!(body.contains("new body"));
}

#[tokio::test]
async fn update_dry_run_does_not_write() {
    let server_install = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/foo/bar/main/registry.json"))
        .respond_with(ResponseTemplate::new(200).set_body_string(REG_OLD))
        .mount(&server_install)
        .await;
    Mock::given(method("GET"))
        .and(path("/foo/bar/main/skills/csv-parse/SKILL.md"))
        .respond_with(ResponseTemplate::new(200).set_body_string(SKILL_OLD))
        .mount(&server_install)
        .await;

    let dir = assert_fs::TempDir::new().unwrap();
    let p = dir.path().to_str().unwrap().to_string();
    let url = "https://github.com/foo/bar.git".to_string();
    let install_base = server_install.uri();
    let p_install = p.clone();
    let url_install = url.clone();
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
            .env("QUAY_GITHUB_BASE_URL", &install_base)
            .args(["--project", &p_install, "add", "csv-parse"])
            .assert()
            .success();
    })
    .await
    .unwrap();

    let server_new = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/foo/bar/main/registry.json"))
        .respond_with(ResponseTemplate::new(200).set_body_string(REG_NEW))
        .mount(&server_new)
        .await;

    let new_base = server_new.uri();
    let p_dry = p.clone();
    tokio::task::spawn_blocking(move || {
        Command::cargo_bin("quay")
            .unwrap()
            .env("QUAY_GITHUB_BASE_URL", &new_base)
            .args(["--project", &p_dry, "update", "--dry-run"])
            .assert()
            .success()
            .stdout(predicates::str::contains("would update csv-parse"));
    })
    .await
    .unwrap();

    let body =
        std::fs::read_to_string(dir.child(".agents/skills/csv-parse/SKILL.md").path()).unwrap();
    assert!(body.contains("1.0.0"));
    assert!(body.contains("old body"));
}
