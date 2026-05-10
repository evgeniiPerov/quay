//! Integration test for `quay add` collision semantics (Task 10).
//!
//! `quay add <skill>` blocks (exits non-zero with a helpful message) when
//! the skill already exists locally.
//!
//! `quay add --force <skill>` overwrites the existing local copy.
//!
//! These tests use the QUAY_GITHUB_BASE_URL debug seam to serve a mock hub,
//! so they are gated behind `cfg(debug_assertions)`.

#![cfg(debug_assertions)]

use assert_cmd::Command;
use assert_fs::prelude::*;
use assert_fs::TempDir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const REGISTRY: &str = r#"{
    "hub": "fixture",
    "generated_at": "2026-05-10T00:00:00Z",
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

/// Build a TOML config string with a single GitHub-style remote pointing to `base_url`.
///
/// Using a `github.com` URL format so the GithubRawFetcher can redirect through
/// QUAY_GITHUB_BASE_URL in debug builds.
fn project_config_with_remote(hub_name: &str, owner: &str, repo: &str) -> String {
    format!(
        "[remotes.{hub_name}]\nurl = \"https://github.com/{owner}/{repo}.git\"\ndefault = true\n"
    )
}

/// `quay add <skill>` errors when the skill already exists and `--force` is absent.
#[tokio::test]
async fn quay_add_blocks_when_skill_already_exists() {
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

    let project = TempDir::new().unwrap();
    let user_cfg = TempDir::new().unwrap();
    // Empty user config — isolates from real ~/.config/quay/config.toml.
    user_cfg.child("config.toml").write_str("").unwrap();

    // Write project config with mock remote.
    project
        .child(".quay/config.toml")
        .write_str(&project_config_with_remote("fixture", "foo", "bar"))
        .unwrap();

    // Pre-install the skill.
    project
        .child(".agents/skills/csv-parse/SKILL.md")
        .write_str("existing content")
        .unwrap();

    let base = server.uri();
    let p = project.path().to_str().unwrap().to_string();
    let uc = user_cfg
        .child("config.toml")
        .path()
        .to_str()
        .unwrap()
        .to_string();

    let output = tokio::task::spawn_blocking(move || {
        Command::cargo_bin("quay")
            .unwrap()
            .env("QUAY_GITHUB_BASE_URL", &base)
            .args(["--project", &p, "--user-config", &uc, "add", "csv-parse"])
            .output()
            .unwrap()
    })
    .await
    .unwrap();

    assert!(
        !output.status.success(),
        "expected non-zero exit when skill already exists"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("already exists") || stderr.contains("force"),
        "expected hint to use --force, got: {stderr}"
    );

    // The existing file should be unchanged.
    let content =
        std::fs::read_to_string(project.path().join(".agents/skills/csv-parse/SKILL.md")).unwrap();
    assert_eq!(
        content, "existing content",
        "existing file must not be overwritten without --force"
    );
}

/// `quay add --force <skill>` overwrites the existing local copy.
#[tokio::test]
async fn quay_add_force_overwrites_existing_skill() {
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

    let project = TempDir::new().unwrap();
    let user_cfg = TempDir::new().unwrap();
    user_cfg.child("config.toml").write_str("").unwrap();

    project
        .child(".quay/config.toml")
        .write_str(&project_config_with_remote("fixture", "foo", "bar"))
        .unwrap();

    // Pre-install the skill with stale content.
    project
        .child(".agents/skills/csv-parse/SKILL.md")
        .write_str("stale content")
        .unwrap();

    let base = server.uri();
    let p = project.path().to_str().unwrap().to_string();
    let uc = user_cfg
        .child("config.toml")
        .path()
        .to_str()
        .unwrap()
        .to_string();

    let output = tokio::task::spawn_blocking(move || {
        Command::cargo_bin("quay")
            .unwrap()
            .env("QUAY_GITHUB_BASE_URL", &base)
            .args([
                "--project",
                &p,
                "--user-config",
                &uc,
                "add",
                "--force",
                "csv-parse",
            ])
            .output()
            .unwrap()
    })
    .await
    .unwrap();

    assert!(
        output.status.success(),
        "expected success with --force; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let content =
        std::fs::read_to_string(project.path().join(".agents/skills/csv-parse/SKILL.md")).unwrap();
    assert_eq!(
        content, SKILL_MD,
        "--force must overwrite the existing file with new content"
    );
}
