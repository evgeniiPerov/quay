//! End-to-end: build a local bare `file://` git hub, point a project at it in
//! DIRECT push mode, add a local skill, and call the run_push seam. Asserts the
//! bare hub received the skill commit.
//!
//! Push correctness in PR/branch modes is covered by
//! `quay-core/tests/pusher_direct_branch.rs`; this test exercises the MCP
//! `quay_push` wiring (config resolution, provider/opener construction,
//! PushResult → PushOutcome mapping) against a real hub.

use quay_core::BumpKind;
use std::path::Path;
use std::process::Command;

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn git: {e}"));
    assert!(
        out.status.success(),
        "git {:?} in {} failed:\n{}",
        args,
        dir.display(),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn quay_push_direct_lands_on_hub() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();

    // 1. Build a bare hub repo seeded with one commit on `main`.
    let hub_work = root.join("hub-work");
    std::fs::create_dir_all(&hub_work).unwrap();
    std::fs::write(hub_work.join("README.md"), b"hub\n").unwrap();
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

    // 2. Project pointed at the hub in direct push mode, with an author identity.
    let project = root.join("project");
    std::fs::create_dir_all(project.join(".quay")).unwrap();
    std::fs::write(
        project.join(".quay/config.toml"),
        format!(
            "[user]\nname = \"Test User\"\nemail = \"test@example.com\"\n\n\
             [install]\ncanonical = \".agents/skills\"\n\n\
             [remotes.h]\nurl = '{hub_url}'\ndefault = true\npush_mode = \"direct\"\n"
        ),
    )
    .unwrap();

    // 3. A local skill to push (the pusher reads `.agents/skills/<name>`).
    let skill_dir = project.join(".agents/skills/csv-parse");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: csv-parse\ndescription: Parse CSV.\nversion: 0.1.0\n---\nbody\n",
    )
    .unwrap();

    // 4. Run the push.
    let server = quay_mcp::test_support::server_at(&project);
    let outcome = server
        .run_push_for_test("csv-parse", None, BumpKind::AsWritten)
        .expect("push succeeds");
    assert!(outcome.ok);
    // Direct mode opens no PR.
    assert!(outcome.url.is_none(), "direct push has no PR url");
    assert!(
        outcome.message.contains("csv-parse"),
        "summary should mention the skill, got: {}",
        outcome.message
    );

    // 5. Assert the bare hub received the skill on `main`.
    let verify = root.join("verify");
    git(
        root,
        &[
            "clone",
            "-q",
            "--branch",
            "main",
            bare.to_str().unwrap(),
            verify.to_str().unwrap(),
        ],
    );
    assert!(
        verify.join("skills/csv-parse/SKILL.md").exists(),
        "skill must exist on the hub after a direct push"
    );
    let log = Command::new("git")
        .arg("-C")
        .arg(&verify)
        .args(["log", "--oneline", "-5"])
        .output()
        .unwrap();
    let log = String::from_utf8_lossy(&log.stdout);
    assert!(
        log.contains("csv-parse"),
        "hub main must contain the skill commit; log:\n{log}"
    );
}
