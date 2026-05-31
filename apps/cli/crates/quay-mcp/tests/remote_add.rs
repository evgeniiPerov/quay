//! Roundtrip: call the quay_remote add seam against a tempdir project with a
//! minimal `.quay/config.toml`, then assert the written remote round-trips back
//! through `Config::load_resolved`.

use quay_core::Config;

#[test]
fn quay_remote_adds_and_round_trips() {
    let tmp = tempfile::tempdir().unwrap();
    let project = tmp.path();

    // Minimal valid project config (the add path reads it via `Config::read`,
    // which requires the file to exist).
    std::fs::create_dir_all(project.join(".quay")).unwrap();
    std::fs::write(
        project.join(".quay/config.toml"),
        "[install]\ncanonical = \".agents/skills\"\n",
    )
    .unwrap();

    let server = quay_mcp::test_support::server_at(project);
    server
        .add_remote_for_test("hub", "https://github.com/org/skills.git", true)
        .expect("add_remote succeeds");

    // The file now contains the remote, as raw text.
    let toml_text = std::fs::read_to_string(project.join(".quay/config.toml")).unwrap();
    assert!(
        toml_text.contains("[remotes.hub]"),
        "expected [remotes.hub] table in config, got:\n{toml_text}"
    );
    assert!(
        toml_text.contains("https://github.com/org/skills.git"),
        "expected the url in config, got:\n{toml_text}"
    );

    // And it round-trips through the resolved config the rest of quay uses.
    let project_config = project.join(".quay/config.toml");
    let cfg = Config::load_resolved(None, Some(&project_config), None).unwrap();
    let remote = cfg.remotes.get("hub").expect("hub remote present");
    assert_eq!(remote.url, "https://github.com/org/skills.git");
    assert!(remote.default, "remote should be marked default");
}

#[test]
fn quay_remote_rejects_duplicate_name() {
    let tmp = tempfile::tempdir().unwrap();
    let project = tmp.path();
    std::fs::create_dir_all(project.join(".quay")).unwrap();
    std::fs::write(
        project.join(".quay/config.toml"),
        "[install]\ncanonical = \".agents/skills\"\n\n[remotes.hub]\nurl = \"https://github.com/org/a.git\"\ndefault = true\n",
    )
    .unwrap();

    let server = quay_mcp::test_support::server_at(project);
    let err = server
        .add_remote_for_test("hub", "https://github.com/org/b.git", false)
        .expect_err("duplicate remote name must error");
    assert!(
        err.to_string().to_lowercase().contains("hub"),
        "error should mention the conflicting name, got: {err}"
    );
}
