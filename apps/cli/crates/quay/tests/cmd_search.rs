#![cfg(debug_assertions)]

use assert_cmd::Command;
use predicates::prelude::PredicateBooleanExt;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const REGISTRY: &str = r#"{
    "hub": "fixture",
    "generated_at": "2026-05-08T00:00:00Z",
    "schema_version": 1,
    "skills": {
        "csv-parse": {
            "version": "1.2.0",
            "description": "Parse CSV files with auto-delimiter detection.",
            "tags": ["data", "parsing"],
            "category": "backend",
            "path": "skills/csv-parse",
            "sha": "abc",
            "files": ["SKILL.md"]
        },
        "json-clean": {
            "version": "0.5.0",
            "description": "Clean JSON content.",
            "tags": ["data"],
            "category": "backend",
            "path": "skills/json-clean",
            "sha": "def",
            "files": ["SKILL.md"]
        }
    }
}"#;

#[tokio::test]
async fn search_filters_by_substring() {
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
            .args(["--project", &p, "search", "csv"])
            .assert()
            .success()
            .stdout(predicates::str::contains("csv-parse"))
            .stdout(
                predicates::str::contains("Parse CSV")
                    .or(predicates::str::contains("auto-delimiter")),
            )
            .stdout(predicates::str::contains("json-clean").not());
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn search_with_no_remotes_errors() {
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
            .args(["--project", &p, "search", "anything"])
            .assert()
            .failure()
            .stderr(predicates::str::contains("no remotes configured"));
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn search_json_output_is_valid() {
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
        let output = Command::cargo_bin("quay")
            .unwrap()
            .env("QUAY_GITHUB_BASE_URL", &base)
            .args(["--project", &p, "--json", "search", ""])
            .output()
            .unwrap();
        assert!(output.status.success());
        let stdout = String::from_utf8(output.stdout).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
        let arr = parsed.as_array().unwrap();
        assert_eq!(arr.len(), 2);
    })
    .await
    .unwrap();
}
