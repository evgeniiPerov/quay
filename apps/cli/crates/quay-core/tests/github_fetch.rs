#![cfg(debug_assertions)]

use quay_core::{GithubRawFetcherWithBase, RegistryFetcher, SkillFileFetcher};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const REGISTRY_JSON: &str = r#"{
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
async fn fetch_registry_and_skill_via_wiremock() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/foo/bar/main/registry.json"))
        .respond_with(ResponseTemplate::new(200).set_body_string(REGISTRY_JSON))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/foo/bar/main/skills/csv-parse/SKILL.md"))
        .respond_with(ResponseTemplate::new(200).set_body_string(SKILL_MD))
        .mount(&server)
        .await;

    let base = server.uri();
    let hub_url = "https://github.com/foo/bar.git";

    // Construct, use, and drop the fetcher entirely inside spawn_blocking
    // so the reqwest blocking client (which owns a mini-runtime) is never
    // dropped from within the async Tokio context.
    let base_clone = base.clone();
    let reg = tokio::task::spawn_blocking(move || {
        let fetcher = GithubRawFetcherWithBase::new("main", base_clone);
        fetcher.fetch(hub_url)
    })
    .await
    .unwrap()
    .unwrap();
    assert!(reg.entry("csv-parse").is_some());

    let bytes = tokio::task::spawn_blocking(move || {
        let fetcher = GithubRawFetcherWithBase::new("main", base);
        fetcher.fetch_file(hub_url, "skills/csv-parse/SKILL.md")
    })
    .await
    .unwrap()
    .unwrap();
    assert_eq!(String::from_utf8(bytes).unwrap(), SKILL_MD);
}

#[tokio::test]
async fn fetch_file_at_specific_sha_via_wiremock() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/foo/bar/abc123def/skills/csv-parse/SKILL.md"))
        .respond_with(ResponseTemplate::new(200).set_body_string("pinned content"))
        .mount(&server)
        .await;

    let base = server.uri();
    let hub_url = "https://github.com/foo/bar.git";
    let bytes = tokio::task::spawn_blocking(move || {
        let fetcher = GithubRawFetcherWithBase::new("main", base);
        fetcher.fetch_file_at(hub_url, "skills/csv-parse/SKILL.md", "abc123def")
    })
    .await
    .unwrap()
    .unwrap();
    assert_eq!(String::from_utf8(bytes).unwrap(), "pinned content");
}
