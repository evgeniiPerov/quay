//! End-to-end: build a local `file://` git hub, point a project at it,
//! and call quay_add. Verifies the skill lands on disk.

use std::process::Command;

fn git(dir: &std::path::Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(dir)
        .status()
        .unwrap();
    assert!(status.success(), "git {:?} failed", args);
}

#[tokio::test]
async fn quay_add_installs_from_local_hub() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    let hub_work = root.join("hub-work");
    std::fs::create_dir_all(hub_work.join("skills/csv-parse")).unwrap();
    std::fs::write(
        hub_work.join("registry.json"),
        r#"{"hub":"t","generated_at":"2026-05-30T00:00:00Z","schema_version":1,
            "skills":{"csv-parse":{"version":"1.0.0","description":"Parse CSV.",
            "tags":[],"path":"skills/csv-parse","sha":"x","files":["SKILL.md"]}}}"#,
    )
    .unwrap();
    std::fs::write(
        hub_work.join("skills/csv-parse/SKILL.md"),
        "---\nname: csv-parse\ndescription: Parse CSV.\nversion: 1.0.0\n---\nbody\n",
    )
    .unwrap();
    // Pin the branch name so the bare clone's default branch is deterministic
    // (avoids host git's main/master default ambiguity).
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

    let server = quay_mcp::test_support::server_at(&project);
    let out = server
        .quay_add_for_test("csv-parse", None, false)
        .expect("add succeeds");
    assert!(out.ok);

    assert!(project.join(".agents/skills/csv-parse/SKILL.md").exists());
}
