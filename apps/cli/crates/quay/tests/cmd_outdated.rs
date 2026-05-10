#![cfg(debug_assertions)]

use assert_cmd::Command;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

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

#[tokio::test]
async fn outdated_lists_stale_skill() {
    let server_install = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/foo/bar/main/registry.json"))
        .respond_with(ResponseTemplate::new(200).set_body_string(REGISTRY_OLD))
        .mount(&server_install)
        .await;
    Mock::given(method("GET"))
        .and(path("/foo/bar/main/skills/csv-parse/SKILL.md"))
        .respond_with(ResponseTemplate::new(200).set_body_string(SKILL_MD))
        .mount(&server_install)
        .await;

    let dir = assert_fs::TempDir::new().unwrap();
    let p = dir.path().to_str().unwrap().to_string();
    let url = "https://github.com/foo/bar.git".to_string();
    let install_base = server_install.uri();

    // Phase 1: install csv-parse@1.0.0.
    let p2 = p.clone();
    let url2 = url.clone();
    tokio::task::spawn_blocking(move || {
        Command::cargo_bin("quay")
            .unwrap()
            .args(["--project", &p2, "init"])
            .assert()
            .success();
        Command::cargo_bin("quay")
            .unwrap()
            .args(["--project", &p2, "remote", "add", "h", &url2, "--default"])
            .assert()
            .success();
        Command::cargo_bin("quay")
            .unwrap()
            .env("QUAY_GITHUB_BASE_URL", &install_base)
            .args(["--project", &p2, "add", "csv-parse"])
            .assert()
            .success();
    })
    .await
    .unwrap();

    // Phase 2: registry now reports 1.2.0; outdated should list the diff.
    let server_check = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/foo/bar/main/registry.json"))
        .respond_with(ResponseTemplate::new(200).set_body_string(REGISTRY_NEW))
        .mount(&server_check)
        .await;

    let check_base = server_check.uri();
    tokio::task::spawn_blocking(move || {
        Command::cargo_bin("quay")
            .unwrap()
            .env("QUAY_GITHUB_BASE_URL", &check_base)
            .args(["--project", &p, "outdated"])
            .assert()
            .success()
            .stdout(predicates::str::contains("csv-parse"))
            .stdout(predicates::str::contains("1.0.0 -> 1.2.0"));
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn outdated_when_no_install_says_no_skills() {
    let dir = assert_fs::TempDir::new().unwrap();
    let p = dir.path().to_str().unwrap().to_string();
    tokio::task::spawn_blocking(move || {
        Command::cargo_bin("quay")
            .unwrap()
            .args(["--project", &p, "init"])
            .assert()
            .success();
        Command::cargo_bin("quay")
            .unwrap()
            .args(["--project", &p, "outdated"])
            .assert()
            .success()
            .stdout(predicates::str::contains("(everything up to date)"));
    })
    .await
    .unwrap();
}
