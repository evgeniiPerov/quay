// These tests rely on the QUAY_GITHUB_BASE_URL test seam, which is only compiled into
// the binary in debug builds (see commands/add.rs and commands/info.rs). In release
// builds the env var is ignored and these tests would hit raw.githubusercontent.com
// for real, so we gate the entire file out of release-mode test runs.
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
            "sha": "deadbeef",
            "files": ["SKILL.md"]
        }
    }
}"#;

const SKILL_MD: &str = "---\nname: csv-parse\ndescription: Parse CSV.\nversion: 1.0.0\n---\nbody\n";

#[tokio::test]
async fn quay_add_installs_skill_from_mock_hub() {
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

    let dir = assert_fs::TempDir::new().unwrap();
    let p = dir.path().to_str().unwrap().to_string();
    // The hub URL must be GitHub-style so parse_owner_repo can extract foo/bar.
    // QUAY_GITHUB_BASE_URL redirects all HTTP requests to the mock server.
    let url = "https://github.com/foo/bar.git".to_string();
    let base = server.uri();

    tokio::task::spawn_blocking(move || {
        Command::cargo_bin("quay")
            .unwrap()
            .args(["--project", &p, "init"])
            .assert()
            .success();
        Command::cargo_bin("quay")
            .unwrap()
            .args(["--project", &p, "remote", "add", "h", &url, "--default"])
            .assert()
            .success();
        Command::cargo_bin("quay")
            .unwrap()
            .env("QUAY_GITHUB_BASE_URL", &base)
            .args(["--project", &p, "add", "csv-parse"])
            .assert()
            .success();
    })
    .await
    .unwrap();

    dir.child(".agents/skills/csv-parse/SKILL.md")
        .assert(predicates::path::exists());
    dir.child(".agents/skills.lock.json")
        .assert(predicates::path::exists());
}

#[tokio::test]
async fn quay_list_shows_installed_skill() {
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

    let dir = assert_fs::TempDir::new().unwrap();
    let p = dir.path().to_str().unwrap().to_string();
    let url = "https://github.com/foo/bar.git".to_string();
    let base = server.uri();

    tokio::task::spawn_blocking(move || {
        Command::cargo_bin("quay")
            .unwrap()
            .args(["--project", &p, "init"])
            .assert()
            .success();
        Command::cargo_bin("quay")
            .unwrap()
            .args(["--project", &p, "remote", "add", "h", &url, "--default"])
            .assert()
            .success();
        Command::cargo_bin("quay")
            .unwrap()
            .env("QUAY_GITHUB_BASE_URL", &base)
            .args(["--project", &p, "add", "csv-parse"])
            .assert()
            .success();
        Command::cargo_bin("quay")
            .unwrap()
            .args(["--project", &p, "list"])
            .assert()
            .success()
            .stdout(predicates::str::contains("csv-parse"))
            .stdout(predicates::str::contains("1.0.0"));
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn quay_remove_uninstalls_skill() {
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

    let dir = assert_fs::TempDir::new().unwrap();
    let p = dir.path().to_str().unwrap().to_string();
    let url = "https://github.com/foo/bar.git".to_string();
    let base = server.uri();

    let p2 = p.clone();
    tokio::task::spawn_blocking(move || {
        Command::cargo_bin("quay")
            .unwrap()
            .args(["--project", &p2, "init"])
            .assert()
            .success();
        Command::cargo_bin("quay")
            .unwrap()
            .args(["--project", &p2, "remote", "add", "h", &url, "--default"])
            .assert()
            .success();
        Command::cargo_bin("quay")
            .unwrap()
            .env("QUAY_GITHUB_BASE_URL", &base)
            .args(["--project", &p2, "add", "csv-parse"])
            .assert()
            .success();
        Command::cargo_bin("quay")
            .unwrap()
            .args(["--project", &p2, "remove", "csv-parse"])
            .assert()
            .success();
        Command::cargo_bin("quay")
            .unwrap()
            .args(["--project", &p2, "list"])
            .assert()
            .success()
            .stdout(predicates::str::contains("(no skills installed)"));
    })
    .await
    .unwrap();

    dir.child(".agents/skills/csv-parse")
        .assert(predicates::path::missing());
}

#[tokio::test]
async fn quay_info_shows_skill_metadata() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/foo/bar/main/registry.json"))
        .respond_with(ResponseTemplate::new(200).set_body_string(REGISTRY))
        .mount(&server)
        .await;

    let dir = assert_fs::TempDir::new().unwrap();
    let p = dir.path().to_str().unwrap().to_string();
    let url = "https://github.com/foo/bar.git".to_string();
    let base = server.uri();

    tokio::task::spawn_blocking(move || {
        Command::cargo_bin("quay")
            .unwrap()
            .args(["--project", &p, "init"])
            .assert()
            .success();
        Command::cargo_bin("quay")
            .unwrap()
            .args(["--project", &p, "remote", "add", "h", &url, "--default"])
            .assert()
            .success();
        Command::cargo_bin("quay")
            .unwrap()
            .env("QUAY_GITHUB_BASE_URL", &base)
            .args(["--project", &p, "info", "csv-parse"])
            .assert()
            .success()
            .stdout(predicates::str::contains("csv-parse"))
            .stdout(predicates::str::contains("1.0.0"))
            .stdout(predicates::str::contains("Parse CSV"));
    })
    .await
    .unwrap();
}
