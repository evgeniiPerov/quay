//! Integration test for GitLab compare-URL fallback.
//!
//! When `glab` is NOT on PATH but the remote is detected as GitLab, `quay push`
//! should fall back to printing a `/-/merge_requests/new` compare URL and exit 0.
//!
//! The test is `#[ignore]` because:
//! 1. Setting `PATH=/nonexistent` also breaks `git`, which the pusher needs.
//!    A proper fix would shadow PATH with a temp dir containing only a `git`
//!    symlink and no `glab`/`gh`, but that is fiddly and environment-specific.
//! 2. Unit-level coverage in `quay-core::providers::gitlab` already tests the
//!    compare-URL shape; this integration test documents intent rather than
//!    providing strictly required coverage.
//!
//! To run manually (on a machine without `glab` installed):
//!     cargo test --manifest-path apps/cli/Cargo.toml \
//!         push_to_gitlab_url_without_glab_prints_compare_url -- --ignored

use assert_cmd::Command;
use assert_fs::prelude::*;
use std::process::Command as StdCommand;

fn quay() -> Command {
    Command::cargo_bin("quay").unwrap()
}

// Shadows PATH with a symlink to a `which`-resolved git, so it is unix-only.
// `#[ignore]` still has to compile, which is what broke the Windows build.
#[cfg(unix)]
#[test]
#[ignore = "requires git on PATH but no glab; use a shadow PATH dir for a reliable setup"]
fn push_to_gitlab_url_without_glab_prints_compare_url() {
    let tmp = assert_fs::TempDir::new().unwrap();
    let project = assert_fs::TempDir::new().unwrap();
    let p = tmp.path().to_str().unwrap();
    let proj = project.path().to_str().unwrap();

    // Set up a "GitLab-shaped" bare repo. The provider is declared explicitly
    // via `--provider gitlab` so URL pattern matching is not required; any
    // file:// URL qualifies.
    let bare = tmp.child("hub.git");
    StdCommand::new("git")
        .args(["init", "--bare"])
        .arg(bare.path())
        .status()
        .unwrap();

    // Seed the bare repo with a registry.json so `push` can operate.
    let seed = tmp.path().join("seed");
    StdCommand::new("git")
        .args(["clone"])
        .arg(bare.path())
        .arg(&seed)
        .status()
        .unwrap();
    std::fs::write(seed.join("registry.json"), "{\"skills\":[]}").unwrap();
    for args in &[
        vec!["-C", seed.to_str().unwrap(), "config", "user.email", "t@t"],
        vec!["-C", seed.to_str().unwrap(), "config", "user.name", "t"],
        vec!["-C", seed.to_str().unwrap(), "add", "registry.json"],
        vec!["-C", seed.to_str().unwrap(), "commit", "-m", "init"],
        vec![
            "-C",
            seed.to_str().unwrap(),
            "push",
            "origin",
            "HEAD:refs/heads/main",
        ],
    ] {
        StdCommand::new("git").args(args).status().unwrap();
    }

    let url = format!("file://{}", bare.path().display());

    // Init project, add remote with explicit gitlab provider.
    quay().args(["--project", p, "init"]).assert().success();
    quay()
        .args([
            "--project",
            p,
            "remote",
            "add",
            "hub",
            &url,
            "--provider",
            "gitlab",
        ])
        .assert()
        .success();

    // Also init the skill project.
    quay().args(["--project", proj, "init"]).assert().success();
    quay()
        .args([
            "--project",
            proj,
            "remote",
            "add",
            "hub",
            &url,
            "--provider",
            "gitlab",
        ])
        .assert()
        .success();
    quay()
        .args(["--project", proj, "create", "my-skill"])
        .assert()
        .success();

    // Build a PATH that has `git` but not `glab`/`gh`.
    // On most Linux systems, git is at /usr/bin/git.
    let shadow_path = tmp.path().join("shadow_bin");
    std::fs::create_dir_all(&shadow_path).unwrap();
    let git_bin = std::process::Command::new("which")
        .arg("git")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "/usr/bin/git".to_string());
    let git_link = shadow_path.join("git");
    let _ = std::os::unix::fs::symlink(&git_bin, &git_link);

    let out = quay()
        .args(["--project", proj, "push", "my-skill"])
        .env("PATH", &shadow_path)
        .output()
        .unwrap();

    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        combined.contains("/-/merge_requests/new"),
        "expected gitlab compare URL in output, got:\n{}",
        combined
    );
}
