//! Integration tests for `quay remote test <name>`.
//! Uses local bare git repos (file:// URLs) so no network access is needed.

use assert_cmd::Command;
use std::path::Path;
use std::process::Command as StdCommand;

fn quay() -> Command {
    Command::cargo_bin("quay").unwrap()
}

/// Create a bare repo with a `registry.json` on `main` and return its path.
fn init_bare_repo_with_registry(dir: &Path, registry_body: &str) -> std::path::PathBuf {
    let bare = dir.join("repo.git");
    StdCommand::new("git")
        .args(["init", "--bare"])
        .arg(&bare)
        .status()
        .unwrap();
    // Explicitly set HEAD → main so shallow-clone fallback finds it.
    StdCommand::new("git")
        .args(["-C"])
        .arg(&bare)
        .args(["symbolic-ref", "HEAD", "refs/heads/main"])
        .status()
        .unwrap();
    let work = dir.join("work");
    StdCommand::new("git")
        .args(["clone"])
        .arg(&bare)
        .arg(&work)
        .status()
        .unwrap();
    std::fs::write(work.join("registry.json"), registry_body).unwrap();
    StdCommand::new("git")
        .args(["-C"])
        .arg(&work)
        .args(["config", "user.email", "t@t"])
        .status()
        .unwrap();
    StdCommand::new("git")
        .args(["-C"])
        .arg(&work)
        .args(["config", "user.name", "t"])
        .status()
        .unwrap();
    StdCommand::new("git")
        .args(["-C"])
        .arg(&work)
        .args(["add", "registry.json"])
        .status()
        .unwrap();
    StdCommand::new("git")
        .args(["-C"])
        .arg(&work)
        .args(["commit", "-m", "init"])
        .status()
        .unwrap();
    StdCommand::new("git")
        .args(["-C"])
        .arg(&work)
        .args(["push", "origin", "HEAD:refs/heads/main"])
        .status()
        .unwrap();
    bare
}

#[test]
fn test_command_succeeds_against_local_bare_repo_with_registry() {
    let tmp = assert_fs::TempDir::new().unwrap();
    let bare = init_bare_repo_with_registry(tmp.path(), "{\"skills\":[]}");
    let url = format!("file://{}", bare.display());
    let p = tmp.path().to_str().unwrap();

    quay().args(["--project", p, "init"]).assert().success();
    quay()
        .args(["--project", p, "remote", "add", "hub", &url])
        .assert()
        .success();

    let out = quay()
        .args(["--project", p, "remote", "test", "hub"])
        .assert()
        .success()
        .get_output()
        .clone();
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(
        stdout.contains('\u{2713}'),
        "expected checkmark (\u{2713}) in stdout, got:\n{}",
        stdout
    );
}

#[test]
fn test_command_fails_when_registry_missing() {
    let tmp = assert_fs::TempDir::new().unwrap();
    let p = tmp.path().to_str().unwrap();

    // Create an empty bare repo (no commits, no registry.json)
    let bare = tmp.path().join("empty.git");
    StdCommand::new("git")
        .args(["init", "--bare"])
        .arg(&bare)
        .status()
        .unwrap();
    let url = format!("file://{}", bare.display());

    quay().args(["--project", p, "init"]).assert().success();
    quay()
        .args(["--project", p, "remote", "add", "hub", &url])
        .assert()
        .success();

    quay()
        .args(["--project", p, "remote", "test", "hub"])
        .assert()
        .failure()
        .code(1);
}
