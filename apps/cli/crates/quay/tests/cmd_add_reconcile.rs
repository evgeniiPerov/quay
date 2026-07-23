//! Non-TTY behavior of the reconcile collision path.
//!
//! The test creates a local bare git "harbor" (no network needed), configures
//! quay to point at it via a `file://` URL, pre-installs a skill with different
//! content to force a collision, then asserts that a non-TTY `quay add`
//! exits non-zero with a hint about `--force` / reconcile.
//!
//! Isolation: every invocation uses its own XDG_CONFIG_HOME + --project to
//! avoid the known cmd_add config-bleed flake set (see project memory).

use assert_cmd::Command;
use std::path::Path;
use std::process;
use tempfile::TempDir;

/// Run `git -C <dir> <args>` and panic if it fails.
fn git(dir: &Path, args: &[&str]) {
    let status = process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .status()
        .expect("git");
    assert!(status.success(), "git {args:?} failed");
}

/// Build a local bare git repo with:
///   - `registry.json` listing skill `foo` at version `1.0.0`
///   - `skills/foo/SKILL.md` with HARBOR_SKILL_BODY
///
/// Returns `(work_tmp, bare_tmp)` — both must stay alive for the test.
fn make_harbor(skill_body: &str) -> (TempDir, TempDir) {
    let work = TempDir::new().unwrap();
    let bare = TempDir::new().unwrap();

    git(bare.path(), &["init", "--bare", "--initial-branch=main"]);
    git(work.path(), &["init", "--initial-branch=main"]);
    git(
        work.path(),
        &["config", "user.email", "quay-test@example.com"],
    );
    git(work.path(), &["config", "user.name", "quay-test"]);

    let registry = r#"{
    "hub": "test-harbor",
    "generated_at": "2026-05-01T00:00:00Z",
    "schema_version": 1,
    "skills": {
        "foo": {
            "version": "1.0.0",
            "description": "Foo skill",
            "tags": [],
            "path": "skills/foo",
            "sha": "deadbeef",
            "files": ["SKILL.md"]
        }
    }
}"#;
    std::fs::write(work.path().join("registry.json"), registry).unwrap();

    let skill_dir = work.path().join("skills/foo");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(skill_dir.join("SKILL.md"), skill_body).unwrap();

    git(work.path(), &["add", "-A"]);
    git(work.path(), &["commit", "-m", "init harbor"]);

    let bare_url = bare.path().to_str().unwrap().to_string();
    git(work.path(), &["remote", "add", "origin", &bare_url]);
    git(work.path(), &["push", "origin", "main:main"]);

    (work, bare)
}

/// Build a quay project config with a single remote `hub` pointing at `url`.
///
/// Single-quoted TOML literal string, not a basic string: on Windows the url
/// carries a temp path like `C:\Users\...`, and inside a basic string `\U` is
/// read as a unicode escape ("too few unicode value digits").
fn project_config_for_url(url: &str) -> String {
    format!("[remotes.hub]\nurl = '{url}'\ndefault = true\n")
}

fn quay() -> Command {
    Command::cargo_bin("quay").unwrap()
}

const HARBOR_SKILL_BODY: &str =
    "---\nname: foo\ndescription: Foo skill\nversion: 1.0.0\n---\nharbor content\n";
const LOCAL_SKILL_BODY: &str =
    "---\nname: foo\ndescription: Foo skill\nversion: 1.0.0\n---\nlocal content differs\n";

/// In non-TTY mode a collision (without `--force`) exits non-zero and hints at
/// `--force` or interactive reconcile.
#[test]
fn collision_non_tty_exits_nonzero_with_hint() {
    let (_work, bare) = make_harbor(HARBOR_SKILL_BODY);
    let bare_url = format!("file://{}", bare.path().display());

    let proj = TempDir::new().unwrap();
    let cfg_home = TempDir::new().unwrap();

    // Write project-level quay config.
    let quay_dir = proj.path().join(".quay");
    std::fs::create_dir_all(&quay_dir).unwrap();
    std::fs::write(
        quay_dir.join("config.toml"),
        project_config_for_url(&bare_url),
    )
    .unwrap();

    // Pre-install the skill with DIFFERENT content (triggers collision).
    let skill_dir = proj.path().join(".agents/skills/foo");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(skill_dir.join("SKILL.md"), LOCAL_SKILL_BODY).unwrap();

    // Provide an isolated (empty) user config so we never read the real one.
    let user_cfg_path = cfg_home.path().join("quay/config.toml");
    std::fs::create_dir_all(user_cfg_path.parent().unwrap()).unwrap();
    std::fs::write(&user_cfg_path, "").unwrap();

    let output = quay()
        .env("XDG_CONFIG_HOME", cfg_home.path())
        .args([
            "--project",
            proj.path().to_str().unwrap(),
            "--user-config",
            user_cfg_path.to_str().unwrap(),
            "add",
            "foo",
        ])
        .output()
        .unwrap();

    assert!(
        !output.status.success(),
        "expected non-zero exit on non-TTY collision without --force"
    );

    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    assert!(
        stderr.contains("--force"),
        "stderr should contain '--force' hint; got stderr={stderr:?}"
    );
    assert!(
        stderr.contains("differs"),
        "stderr should contain 'differs'; got stderr={stderr:?}"
    );
}

/// `quay add --force foo` still overwrites unconditionally (reconcile is
/// bypassed on the force path).
#[test]
fn force_still_overwrites_without_reconcile() {
    let (_work, bare) = make_harbor(HARBOR_SKILL_BODY);
    let bare_url = format!("file://{}", bare.path().display());

    let proj = TempDir::new().unwrap();
    let cfg_home = TempDir::new().unwrap();

    let quay_dir = proj.path().join(".quay");
    std::fs::create_dir_all(&quay_dir).unwrap();
    std::fs::write(
        quay_dir.join("config.toml"),
        project_config_for_url(&bare_url),
    )
    .unwrap();

    let skill_dir = proj.path().join(".agents/skills/foo");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(skill_dir.join("SKILL.md"), LOCAL_SKILL_BODY).unwrap();

    let user_cfg_path = cfg_home.path().join("quay/config.toml");
    std::fs::create_dir_all(user_cfg_path.parent().unwrap()).unwrap();
    std::fs::write(&user_cfg_path, "").unwrap();

    let output = quay()
        .env("XDG_CONFIG_HOME", cfg_home.path())
        .args([
            "--project",
            proj.path().to_str().unwrap(),
            "--user-config",
            user_cfg_path.to_str().unwrap(),
            "add",
            "--force",
            "foo",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "expected success with --force; stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    let on_disk = std::fs::read_to_string(proj.path().join(".agents/skills/foo/SKILL.md")).unwrap();
    assert_eq!(
        on_disk, HARBOR_SKILL_BODY,
        "--force must overwrite with harbor content"
    );
}

/// Adding a skill that is NOT present locally must succeed via the normal
/// install path and must NOT enter the reconcile/collision path at all.
///
/// This is the fast-path regression test: the reconcile feature must be
/// entirely invisible when there is no collision.
#[test]
fn fresh_add_does_not_trigger_reconcile() {
    let (_work, bare) = make_harbor(HARBOR_SKILL_BODY);
    let bare_url = format!("file://{}", bare.path().display());

    let proj = TempDir::new().unwrap();
    let cfg_home = TempDir::new().unwrap();

    // Write project-level quay config pointing at our local bare harbor.
    let quay_dir = proj.path().join(".quay");
    std::fs::create_dir_all(&quay_dir).unwrap();
    std::fs::write(
        quay_dir.join("config.toml"),
        project_config_for_url(&bare_url),
    )
    .unwrap();

    // Deliberately do NOT pre-install `foo` locally — there is no collision.

    // Isolated (empty) user config so we never read the real one.
    let user_cfg_path = cfg_home.path().join("quay/config.toml");
    std::fs::create_dir_all(user_cfg_path.parent().unwrap()).unwrap();
    std::fs::write(&user_cfg_path, "").unwrap();

    let output = quay()
        .env("XDG_CONFIG_HOME", cfg_home.path())
        .args([
            "--project",
            proj.path().to_str().unwrap(),
            "--user-config",
            user_cfg_path.to_str().unwrap(),
            "add",
            "foo",
        ])
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();

    // Command must succeed: fresh install, no collision.
    assert!(
        output.status.success(),
        "expected exit 0 for fresh install; stderr={stderr:?} stdout={stdout:?}"
    );

    // The skill must now be installed on disk.
    let installed_path = proj.path().join(".agents/skills/foo/SKILL.md");
    assert!(
        installed_path.exists(),
        "expected foo/SKILL.md to be installed; proj={:?}",
        proj.path()
    );
    let on_disk = std::fs::read_to_string(&installed_path).unwrap();
    assert_eq!(
        on_disk, HARBOR_SKILL_BODY,
        "installed content must match harbor content"
    );

    // The reconcile path must NOT have been entered.
    // Both of these strings only appear when handle_collision() runs.
    assert!(
        !stderr.contains("could not reach harbor to compare"),
        "reconcile warning must not appear on a fresh install; stderr={stderr:?}"
    );
    assert!(
        !stderr.contains("differs from harbor"),
        "collision error must not appear on a fresh install; stderr={stderr:?}"
    );
}

/// When the local skill content is byte-identical to the harbor HEAD copy,
/// `quay add` exits with status 0 and reports "identical to harbor — nothing to do."
#[test]
fn identical_content_is_noop() {
    // Harbor contains HARBOR_SKILL_BODY; local install is the same content.
    let (_work, bare) = make_harbor(HARBOR_SKILL_BODY);
    let bare_url = format!("file://{}", bare.path().display());

    let proj = TempDir::new().unwrap();
    let cfg_home = TempDir::new().unwrap();

    let quay_dir = proj.path().join(".quay");
    std::fs::create_dir_all(&quay_dir).unwrap();
    std::fs::write(
        quay_dir.join("config.toml"),
        project_config_for_url(&bare_url),
    )
    .unwrap();

    // Pre-install the skill with IDENTICAL content to the harbor.
    let skill_dir = proj.path().join(".agents/skills/foo");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(skill_dir.join("SKILL.md"), HARBOR_SKILL_BODY).unwrap();

    let user_cfg_path = cfg_home.path().join("quay/config.toml");
    std::fs::create_dir_all(user_cfg_path.parent().unwrap()).unwrap();
    std::fs::write(&user_cfg_path, "").unwrap();

    let output = quay()
        .env("XDG_CONFIG_HOME", cfg_home.path())
        .args([
            "--project",
            proj.path().to_str().unwrap(),
            "--user-config",
            user_cfg_path.to_str().unwrap(),
            "add",
            "foo",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "expected exit 0 for identical content; stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    assert!(
        stdout.contains("identical to harbor — nothing to do."),
        "stdout should report identical verdict; got stdout={stdout:?}"
    );

    // File must be unchanged.
    let on_disk = std::fs::read_to_string(proj.path().join(".agents/skills/foo/SKILL.md")).unwrap();
    assert_eq!(
        on_disk, HARBOR_SKILL_BODY,
        "identical content must leave the local file unchanged"
    );
}
