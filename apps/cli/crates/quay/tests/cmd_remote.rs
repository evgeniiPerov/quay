use assert_cmd::Command;

fn quay() -> Command {
    Command::cargo_bin("quay").unwrap()
}

#[test]
fn add_then_list_then_remove() {
    let dir = assert_fs::TempDir::new().unwrap();
    let p = dir.path().to_str().unwrap();

    quay().args(["--project", p, "init"]).assert().success();
    quay()
        .args([
            "--project",
            p,
            "remote",
            "add",
            "my-hub",
            "https://github.com/foo/bar.git",
            "--default",
        ])
        .assert()
        .success();
    quay()
        .args(["--project", p, "remote", "list"])
        .assert()
        .success()
        .stdout(predicates::str::contains("my-hub"))
        .stdout(predicates::str::contains("https://github.com/foo/bar.git"))
        .stdout(predicates::str::contains("[default]"));
    quay()
        .args(["--project", p, "remote", "remove", "my-hub"])
        .assert()
        .success();
    quay()
        .args(["--project", p, "remote", "list"])
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
