use assert_cmd::Command;
use assert_fs::prelude::*;

fn init_bare_with_main(bare: &std::path::Path) {
    std::process::Command::new("git")
        .arg("init")
        .arg("--bare")
        .arg(bare)
        .output()
        .unwrap();
    // Seed the bare repo with an initial commit on `main` so quay can clone it.
    let work = tempfile::tempdir().unwrap();
    std::process::Command::new("git")
        .arg("clone")
        .arg(bare)
        .arg(work.path())
        .output()
        .unwrap();
    std::fs::write(work.path().join("README.md"), b"hub\n").unwrap();
    std::process::Command::new("git")
        .arg("-C")
        .arg(work.path())
        .arg("checkout")
        .arg("-B")
        .arg("main")
        .output()
        .unwrap();
    std::process::Command::new("git")
        .arg("-C")
        .arg(work.path())
        .arg("add")
        .arg("-A")
        .output()
        .unwrap();
    std::process::Command::new("git")
        .arg("-C")
        .arg(work.path())
        .arg("-c")
        .arg("user.email=t@e")
        .arg("-c")
        .arg("user.name=Test")
        .arg("commit")
        .arg("-m")
        .arg("init")
        .output()
        .unwrap();
    std::process::Command::new("git")
        .arg("-C")
        .arg(work.path())
        .arg("push")
        .arg("-u")
        .arg("origin")
        .arg("main")
        .output()
        .unwrap();
}

#[test]
fn push_creates_branch_in_bare_repo() {
    let tmp = assert_fs::TempDir::new().unwrap();
    let bare = tmp.child("hub.git");
    init_bare_with_main(bare.path());

    let project = tmp.child("project");
    std::fs::create_dir_all(project.path()).unwrap();
    let p = project.path().to_str().unwrap();

    Command::cargo_bin("quay")
        .unwrap()
        .args(["--project", p, "init"])
        .assert()
        .success();
    let bare_url = bare.path().to_str().unwrap();
    Command::cargo_bin("quay")
        .unwrap()
        .args(["--project", p, "remote", "add", "h", bare_url, "--default"])
        .assert()
        .success();

    // Set up author identity in the project config so the pusher knows the author.
    // The [user] section already exists (written by `init`), so we replace the
    // [user] header + empty block with the header + populated fields.
    let cfg_path = project.child(".quay/config.toml");
    let cfg_text = std::fs::read_to_string(cfg_path.path()).unwrap();
    let cfg_text = cfg_text.replace(
        "[user]\n",
        "[user]\nname = \"Alice\"\nemail = \"alice@example.com\"\n",
    );
    std::fs::write(cfg_path.path(), cfg_text).unwrap();

    // Create a skill, fill in description, push.
    Command::cargo_bin("quay")
        .unwrap()
        .args(["--project", p, "create", "csv-parse"])
        .assert()
        .success();
    let md = project.child(".agents/skills/csv-parse/SKILL.md");
    let body = std::fs::read_to_string(md.path())
        .unwrap()
        .replace("description: \n", "description: Parse CSV\n");
    std::fs::write(md.path(), body).unwrap();

    Command::cargo_bin("quay")
        .unwrap()
        .args(["--project", p, "push", "csv-parse"])
        .assert()
        .success()
        .stdout(predicates::str::contains("pushed quay/csv-parse-0.1.0"));

    // Verify the branch landed in the bare repo.
    let lsremote = std::process::Command::new("git")
        .arg("ls-remote")
        .arg(bare.path())
        .arg("quay/csv-parse-0.1.0")
        .output()
        .unwrap();
    let s = String::from_utf8_lossy(&lsremote.stdout);
    assert!(
        s.contains("refs/heads/quay/csv-parse-0.1.0"),
        "branch not pushed to bare repo: {}",
        s
    );
}
