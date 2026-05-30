//! Integration test: `quay rebuild-registry` indexes nested skill files and
//! operates on the remote's configured `direct_branch` (not the default branch).

use assert_cmd::Command;
use assert_fs::prelude::*;
use std::process::Command as Git;

fn git(args: &[&str]) {
    let status = Git::new("git").args(args).status().unwrap();
    assert!(status.success(), "git {:?} failed", args);
}

#[test]
fn rebuild_registry_indexes_nested_files_on_direct_branch() {
    let tmp = assert_fs::TempDir::new().unwrap();
    let bare = tmp.child("hub.git");
    let seed = tmp.child("seed");
    let project = assert_fs::TempDir::new().unwrap();
    let user_cfg = assert_fs::TempDir::new().unwrap();

    // 1. Bare repo, default branch `main` with a stale registry.json.
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

    // 2. develop branch with a NESTED skill (SKILL.md + scripts/ + agents/).
    git(&["-C", seed_p, "checkout", "-b", "develop"]);
    let skill = seed.path().join("skills/demo");
    std::fs::create_dir_all(skill.join("scripts")).unwrap();
    std::fs::create_dir_all(skill.join("agents")).unwrap();
    std::fs::write(
        skill.join("SKILL.md"),
        "---\nname: demo\ndescription: d\nversion: 0.1.0\n---\nbody\n",
    )
    .unwrap();
    std::fs::write(skill.join("scripts/sync.mjs"), "code").unwrap();
    std::fs::write(skill.join("agents/openai.yaml"), "cfg").unwrap();
    git(&["-C", seed_p, "add", "."]);
    git(&["-C", seed_p, "commit", "-m", "add nested skill on develop"]);
    git(&["-C", seed_p, "push", "origin", "develop"]);

    // 3. Project config: remote on develop, direct push mode.
    let url = format!("file://{}", bare.path().display());
    user_cfg.child("config.toml").write_str("").unwrap();
    project
        .child(".quay/config.toml")
        .write_str(&format!(
            "[user]\nemail = \"t@t\"\nname = \"t\"\n\n\
             [remotes.hub]\nurl = \"{url}\"\ndefault = true\n\
             push_mode = \"direct\"\ndirect_branch = \"develop\"\n"
        ))
        .unwrap();

    // 4. Run rebuild-registry.
    Command::cargo_bin("quay")
        .unwrap()
        .args([
            "--user-config",
            user_cfg.child("config.toml").path().to_str().unwrap(),
            "--project",
            project.path().to_str().unwrap(),
            "rebuild-registry",
        ])
        .assert()
        .success();

    // 5. Clone develop fresh and assert registry lists the nested files.
    let check = tmp.child("check");
    git(&[
        "clone",
        "-b",
        "develop",
        "--single-branch",
        bare.path().to_str().unwrap(),
        check.path().to_str().unwrap(),
    ]);
    let text = std::fs::read_to_string(check.path().join("registry.json")).unwrap();
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    let files = &v["skills"]["demo"]["files"];
    assert_eq!(
        files,
        &serde_json::json!(["SKILL.md", "agents/openai.yaml", "scripts/sync.mjs"]),
        "registry.json on develop must list nested files; got {files}"
    );
}
