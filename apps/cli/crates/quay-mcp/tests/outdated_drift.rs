//! An agent calling `quay_outdated` must learn about a hub edit that shipped
//! without a version bump — otherwise it reads "no upgrades" and moves on with
//! a stale copy. Local `file://` hub, skill written straight to disk.

use std::process::Command;

fn git(dir: &std::path::Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(dir)
        .status()
        .unwrap();
    assert!(status.success(), "git {:?} failed", args);
}

/// Registry advertising csv-parse at the same version the project has
/// installed, with a content_hash that cannot match any real folder.
const REGISTRY: &str = r#"{"hub":"t","generated_at":"2026-05-30T00:00:00Z","schema_version":1,
    "skills":{"csv-parse":{"version":"1.0.0","description":"Parse CSV.",
    "tags":[],"path":"skills/csv-parse","sha":"x","files":["SKILL.md"],
    "source_format":"frontmatter",
    "content_hash":"0000000000000000000000000000000000000000000000000000000000000000"}}}"#;

const SKILL_MD: &str = "---\nname: csv-parse\ndescription: Parse CSV.\nversion: 1.0.0\n---\nbody\n";

#[test]
fn quay_outdated_reports_content_drift_at_an_unchanged_version() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    let hub_work = root.join("hub-work");
    std::fs::create_dir_all(hub_work.join("skills/csv-parse")).unwrap();
    std::fs::write(hub_work.join("registry.json"), REGISTRY).unwrap();
    std::fs::write(hub_work.join("skills/csv-parse/SKILL.md"), SKILL_MD).unwrap();
    git(&hub_work, &["init", "-q", "-b", "main"]);
    git(&hub_work, &["config", "user.email", "t@t"]);
    git(&hub_work, &["config", "user.name", "t"]);
    git(&hub_work, &["add", "."]);
    git(&hub_work, &["commit", "-q", "-m", "seed"]);

    let bare = root.join("hub.git");
    git(
        root,
        &[
            "clone",
            "-q",
            "--bare",
            hub_work.to_str().unwrap(),
            bare.to_str().unwrap(),
        ],
    );
    let hub_url = format!("file://{}", bare.display());

    let project = root.join("project");
    std::fs::create_dir_all(project.join(".quay")).unwrap();
    std::fs::write(
        project.join(".quay/config.toml"),
        format!("[install]\ncanonical = \".agents/skills\"\n\n[remotes.h]\nurl = '{hub_url}'\ndefault = true\n"),
    )
    .unwrap();
    let skill_dir = project.join(".agents/skills/csv-parse");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(skill_dir.join("SKILL.md"), SKILL_MD).unwrap();

    let server = quay_mcp::test_support::server_at(&project);
    let report = server.quay_outdated_for_test().expect("outdated succeeds");

    let row = report
        .outdated
        .iter()
        .find(|r| r.name == "csv-parse")
        .expect("csv-parse must be reported despite the version being unchanged");
    assert!(
        row.content_drift,
        "the row has to say *why* it is listed, or an agent cannot tell a \
         drift row from a semver upgrade"
    );
    assert_eq!(row.available, "1.0.0");
}
