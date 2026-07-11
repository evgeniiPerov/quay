use assert_cmd::Command;

fn quay() -> Command {
    Command::cargo_bin("quay").unwrap()
}

#[test]
fn add_then_list_then_remove() {
    let dir = assert_fs::TempDir::new().unwrap();
    let p = dir.path().to_str().unwrap();
    // Isolate the host's ~/.config/quay so its remotes don't bleed into the
    // "(no remotes configured)" assertion below.
    let user_cfg = dir.path().join("user.toml");
    std::fs::write(&user_cfg, "").unwrap();
    let uc = user_cfg.to_str().unwrap();
    let run = |args: &[&str]| {
        let mut c = quay();
        c.env("XDG_CONFIG_HOME", dir.path())
            .args(["--project", p, "--user-config", uc]);
        c.args(args);
        c
    };

    run(&["init"]).assert().success();
    run(&[
        "remote",
        "add",
        "my-hub",
        "https://github.com/foo/bar.git",
        "--default",
    ])
    .assert()
    .success();
    run(&["remote", "list"])
        .assert()
        .success()
        .stdout(predicates::str::contains("my-hub"))
        .stdout(predicates::str::contains("https://github.com/foo/bar.git"))
        .stdout(predicates::str::contains("[default]"));
    run(&["remote", "remove", "my-hub"]).assert().success();
    run(&["remote", "list"])
        .assert()
        .success()
        .stdout(predicates::str::contains("(no remotes configured)"));
}

#[test]
fn add_duplicate_fails() {
    let dir = assert_fs::TempDir::new().unwrap();
    let p = dir.path().to_str().unwrap();
    quay().args(["--project", p, "init"]).assert().success();
    quay()
        .args([
            "--project",
            p,
            "remote",
            "add",
            "h",
            "https://github.com/x/y.git",
        ])
        .assert()
        .success();
    quay()
        .args([
            "--project",
            p,
            "remote",
            "add",
            "h",
            "https://github.com/x/y.git",
        ])
        .assert()
        .failure()
        .stderr(predicates::str::contains("already exists"));
}

#[test]
fn warns_when_multiple_remotes_marked_default() {
    let dir = assert_fs::TempDir::new().unwrap();
    let p = dir.path().to_str().unwrap();
    let user_cfg = dir.path().join("user.toml");
    std::fs::write(&user_cfg, "").unwrap();

    quay()
        .args([
            "--project",
            p,
            "--user-config",
            user_cfg.to_str().unwrap(),
            "init",
        ])
        .assert()
        .success();

    // Hand-write a project config with two defaults (the CLI normally clears
    // the prior default, but hand-edited / migrated configs can still hit this).
    let cfg = r#"[remotes.alpha]
url = "https://github.com/a/b.git"
default = true

[remotes.beta]
url = "https://github.com/c/d.git"
default = true
"#;
    std::fs::write(format!("{}/.quay/config.toml", p), cfg).unwrap();

    quay()
        .args([
            "--project",
            p,
            "--user-config",
            user_cfg.to_str().unwrap(),
            "remote",
            "list",
        ])
        .assert()
        .success()
        .stderr(predicates::str::contains("2 remotes marked as default"))
        .stderr(predicates::str::contains("alpha"))
        .stderr(predicates::str::contains("beta"));
}

#[test]
fn second_default_unsets_first() {
    let dir = assert_fs::TempDir::new().unwrap();
    let p = dir.path().to_str().unwrap();
    quay().args(["--project", p, "init"]).assert().success();
    quay()
        .args([
            "--project",
            p,
            "remote",
            "add",
            "first",
            "https://github.com/a/b.git",
            "--default",
        ])
        .assert()
        .success();
    quay()
        .args([
            "--project",
            p,
            "remote",
            "add",
            "second",
            "https://github.com/c/d.git",
            "--default",
        ])
        .assert()
        .success();
    quay()
        .args(["--project", p, "remote", "list"])
        .assert()
        .success()
        .stdout(predicates::str::contains("first"))
        .stdout(predicates::str::contains("second"))
        // Only `second` should carry the [default] tag in the output.
        .stdout(predicates::str::is_match(r"second\s+\S+\s+\[default\]").unwrap());
}
