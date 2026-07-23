//! Integration test: `quay remove <skill> --remote` deletes the skill from the
//! default remote's configured `direct_branch` (not the default branch) and
//! drops its registry entry, leaving any local copy untouched.

use assert_cmd::Command;
use assert_fs::prelude::*;
use std::process::Command as Git;

fn git(args: &[&str]) {
    let status = Git::new("git").args(args).status().unwrap();
    assert!(status.success(), "git {:?} failed", args);
}

#[test]
fn remove_remote_deletes_skill_on_direct_branch() {
    let tmp = assert_fs::TempDir::new().unwrap();
    let bare = tmp.child("hub.git");
    let seed = tmp.child("seed");
    let project = assert_fs::TempDir::new().unwrap();
    let user_cfg = assert_fs::TempDir::new().unwrap();

    // Bare repo, default branch `main` (stale empty registry).
    git(&[
        "init",
        "--bare",
        "-b",
        "main",
        bare.path().to_str().unwrap(),
    ]);
    git(&[
        "clone",
        bare.path().to_str().unwrap(),
        seed.path().to_str().unwrap(),
    ]);
    let seed_p = seed.path().to_str().unwrap();
    git(&["-C", seed_p, "config", "user.email", "t@t"]);
    git(&["-C", seed_p, "config", "user.name", "t"]);
    std::fs::write(
        seed.path().join("registry.json"),
        "{\"hub\":\"h\",\"generated_at\":\"x\",\"schema_version\":1,\"skills\":{}}",
    )
    .unwrap();
    git(&["-C", seed_p, "add", "."]);
    git(&["-C", seed_p, "commit", "-m", "init main"]);
    git(&["-C", seed_p, "push", "origin", "main"]);

    // develop branch with skill `demo` + a registry entry for it.
    git(&["-C", seed_p, "checkout", "-b", "develop"]);
    let skill = seed.path().join("skills/demo");
    std::fs::create_dir_all(&skill).unwrap();
    std::fs::write(
        skill.join("SKILL.md"),
        "---\nname: demo\ndescription: d\nversion: 0.1.0\n---\nbody\n",
    )
    .unwrap();
    std::fs::write(
        seed.path().join("registry.json"),
        "{\"hub\":\"h\",\"generated_at\":\"x\",\"schema_version\":1,\"skills\":{\
\"demo\":{\"version\":\"0.1.0\",\"description\":\"d\",\"tags\":[],\
\"path\":\"skills/demo\",\"sha\":\"x\",\"files\":[\"SKILL.md\"]}}}",
    )
    .unwrap();
    git(&["-C", seed_p, "add", "."]);
    git(&["-C", seed_p, "commit", "-m", "add demo on develop"]);
    git(&["-C", seed_p, "push", "origin", "develop"]);

    // Project config: default remote on develop, direct push mode.
    let url = format!("file://{}", bare.path().display());
    user_cfg.child("config.toml").write_str("").unwrap();
    project
        .child(".quay/config.toml")
        .write_str(&format!(
            "[user]\nemail = \"t@t\"\nname = \"t\"\n\n\
             [remotes.hub]\nurl = '{url}'\ndefault = true\n\
             push_mode = \"direct\"\ndirect_branch = \"develop\"\n"
        ))
        .unwrap();

    // Run: remove demo from the hub only.
    Command::cargo_bin("quay")
        .unwrap()
        .args([
            "--user-config",
            user_cfg.child("config.toml").path().to_str().unwrap(),
            "--project",
            project.path().to_str().unwrap(),
            "remove",
            "demo",
            "--remote",
        ])
        .assert()
        .success();

    // Clone develop fresh: skill dir gone, registry entry gone.
    let check = tmp.child("check");
    git(&[
        "clone",
        "-b",
        "develop",
        "--single-branch",
        bare.path().to_str().unwrap(),
        check.path().to_str().unwrap(),
    ]);
    assert!(
        !check.path().join("skills/demo").exists(),
        "skills/demo must be deleted on develop"
    );
    let text = std::fs::read_to_string(check.path().join("registry.json")).unwrap();
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert!(
        v["skills"].get("demo").is_none(),
        "registry.json on develop must no longer list demo; got {}",
        v["skills"]
    );
}
