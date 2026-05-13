use assert_cmd::Command;
use assert_fs::prelude::*;

fn write_user(dir: &assert_fs::TempDir, contents: &str) -> std::path::PathBuf {
    let p = dir.child("user.toml");
    std::fs::write(p.path(), contents).unwrap();
    p.path().to_path_buf()
}

#[test]
fn profile_list_prints_marker_for_active() {
    let tmp = assert_fs::TempDir::new().unwrap();
    let user = write_user(
        &tmp,
        r#"
            active_profile = "work"
            [profiles.work.user]
            email = "e@work"
            [profiles.personal.user]
            email = "e@home"
        "#,
    );
    let project = tmp.child("project");
    std::fs::create_dir_all(project.path()).unwrap();
    Command::cargo_bin("quay")
        .unwrap()
        .args([
            "--user-config",
            user.to_str().unwrap(),
            "--project",
            project.path().to_str().unwrap(),
            "profile",
            "list",
        ])
        .assert()
        .success()
        .stdout(predicates::str::contains("* work"))
        .stdout(predicates::str::contains("  personal"));
}

#[test]
fn profile_current_prints_active_name() {
    let tmp = assert_fs::TempDir::new().unwrap();
    let user = write_user(
        &tmp,
        r#"
            active_profile = "work"
            [profiles.work.user]
            email = "e@work"
        "#,
    );
    let project = tmp.child("project");
    std::fs::create_dir_all(project.path()).unwrap();
    Command::cargo_bin("quay")
        .unwrap()
        .args([
            "--user-config",
            user.to_str().unwrap(),
            "--project",
            project.path().to_str().unwrap(),
            "profile",
            "current",
        ])
        .assert()
        .success()
        .stdout(predicates::str::starts_with("work"));
}

#[test]
fn profile_list_empty_when_no_config() {
    let tmp = assert_fs::TempDir::new().unwrap();
    let user = write_user(&tmp, "");
    let project = tmp.child("project");
    std::fs::create_dir_all(project.path()).unwrap();
    Command::cargo_bin("quay")
        .unwrap()
        .args([
            "--user-config",
            user.to_str().unwrap(),
            "--project",
            project.path().to_str().unwrap(),
            "profile",
            "list",
        ])
        .assert()
        .success()
        .stdout(predicates::str::contains("(no profiles configured"));
}

#[test]
fn profile_add_first_profile_makes_it_active() {
    let tmp = assert_fs::TempDir::new().unwrap();
    let user = write_user(&tmp, "");
    let project = tmp.child("project");
    std::fs::create_dir_all(project.path()).unwrap();
    Command::cargo_bin("quay")
        .unwrap()
        .args([
            "--user-config",
            user.to_str().unwrap(),
            "--project",
            project.path().to_str().unwrap(),
            "profile",
            "add",
            "work",
            "--email",
            "e@work",
        ])
        .assert()
        .success()
        .stdout(predicates::str::contains("added profile 'work'"))
        .stdout(predicates::str::contains("set as active"));
    let written = std::fs::read_to_string(&user).unwrap();
    assert!(written.contains("active_profile = \"work\""));
    assert!(written.contains("[profiles.work.user]"));
}

#[test]
fn profile_add_with_remote_seeds_a_default() {
    let tmp = assert_fs::TempDir::new().unwrap();
    let user = write_user(&tmp, "");
    let project = tmp.child("project");
    std::fs::create_dir_all(project.path()).unwrap();
    Command::cargo_bin("quay")
        .unwrap()
        .args([
            "--user-config",
            user.to_str().unwrap(),
            "--project",
            project.path().to_str().unwrap(),
            "profile",
            "add",
            "personal",
            "--email",
            "e@home",
            "--remote",
            "my-pool=https://github.com/me/skills.git",
        ])
        .assert()
        .success();
    let written = std::fs::read_to_string(&user).unwrap();
    assert!(written.contains("[profiles.personal.remotes.my-pool]"));
    assert!(written.contains("default = true"));
}

#[test]
fn profile_add_rejects_duplicate_name() {
    let tmp = assert_fs::TempDir::new().unwrap();
    let user = write_user(
        &tmp,
        r#"
            active_profile = "work"
            [profiles.work.user]
            email = "e@work"
        "#,
    );
    let project = tmp.child("project");
    std::fs::create_dir_all(project.path()).unwrap();
    Command::cargo_bin("quay")
        .unwrap()
        .args([
            "--user-config",
            user.to_str().unwrap(),
            "--project",
            project.path().to_str().unwrap(),
            "profile",
            "add",
            "work",
        ])
        .assert()
        .failure()
        .stderr(predicates::str::contains("already exists"));
}

#[test]
fn profile_use_changes_active() {
    let tmp = assert_fs::TempDir::new().unwrap();
    let user = write_user(
        &tmp,
        r#"
            active_profile = "work"
            [profiles.work.user]
            email = "e@work"
            [profiles.personal.user]
            email = "e@home"
        "#,
    );
    let project = tmp.child("project");
    std::fs::create_dir_all(project.path()).unwrap();
    Command::cargo_bin("quay")
        .unwrap()
        .args([
            "--user-config",
            user.to_str().unwrap(),
            "--project",
            project.path().to_str().unwrap(),
            "profile",
            "use",
            "personal",
        ])
        .assert()
        .success();
    let written = std::fs::read_to_string(&user).unwrap();
    assert!(written.contains("active_profile = \"personal\""));
}

#[test]
fn profile_remove_picks_new_active() {
    let tmp = assert_fs::TempDir::new().unwrap();
    let user = write_user(
        &tmp,
        r#"
            active_profile = "work"
            [profiles.work.user]
            email = "e@work"
            [profiles.personal.user]
            email = "e@home"
        "#,
    );
    let project = tmp.child("project");
    std::fs::create_dir_all(project.path()).unwrap();
    Command::cargo_bin("quay")
        .unwrap()
        .args([
            "--user-config",
            user.to_str().unwrap(),
            "--project",
            project.path().to_str().unwrap(),
            "profile",
            "remove",
            "work",
        ])
        .assert()
        .success();
    let written = std::fs::read_to_string(&user).unwrap();
    assert!(written.contains("active_profile = \"personal\""));
    assert!(!written.contains("[profiles.work"));
}

#[test]
fn profile_remove_refuses_when_only_one() {
    let tmp = assert_fs::TempDir::new().unwrap();
    let user = write_user(
        &tmp,
        r#"
            active_profile = "only"
            [profiles.only.user]
            email = "e@x"
        "#,
    );
    let project = tmp.child("project");
    std::fs::create_dir_all(project.path()).unwrap();
    Command::cargo_bin("quay")
        .unwrap()
        .args([
            "--user-config",
            user.to_str().unwrap(),
            "--project",
            project.path().to_str().unwrap(),
            "profile",
            "remove",
            "only",
        ])
        .assert()
        .failure()
        .stderr(predicates::str::contains("only profile"));
}

#[test]
fn profile_show_prints_active_when_no_arg() {
    let tmp = assert_fs::TempDir::new().unwrap();
    let user = write_user(
        &tmp,
        r#"
            active_profile = "work"
            [profiles.work.user]
            email = "e@work"
            [profiles.work.remotes.h]
            url = "https://x/y.git"
            default = true
        "#,
    );
    let project = tmp.child("project");
    std::fs::create_dir_all(project.path()).unwrap();
    Command::cargo_bin("quay")
        .unwrap()
        .args([
            "--user-config",
            user.to_str().unwrap(),
            "--project",
            project.path().to_str().unwrap(),
            "profile",
            "show",
        ])
        .assert()
        .success()
        .stdout(predicates::str::contains("profile: work"))
        .stdout(predicates::str::contains("e@work"))
        .stdout(predicates::str::contains("* h"));
}

#[test]
fn profile_show_reveals_provider_push_mode_and_direct_branch() {
    let tmp = assert_fs::TempDir::new().unwrap();
    let user = write_user(
        &tmp,
        r#"
            active_profile = "azure-work"
            [profiles.azure-work.user]
            email = "you@example.com"
            [profiles.azure-work.remotes.harbour]
            url = "https://dev.azure.com/example-org/Tooling/_git/frontend-harbour"
            default = true
            provider = "azuredevops"
            push_mode = "direct"
            direct_branch = "develop"
        "#,
    );
    let project = tmp.child("project");
    std::fs::create_dir_all(project.path()).unwrap();
    Command::cargo_bin("quay")
        .unwrap()
        .args([
            "--user-config",
            user.to_str().unwrap(),
            "--project",
            project.path().to_str().unwrap(),
            "profile",
            "show",
        ])
        .assert()
        .success()
        .stdout(predicates::str::contains("profile: azure-work"))
        .stdout(predicates::str::contains("* harbour"))
        .stdout(predicates::str::contains("provider: azuredevops"))
        .stdout(predicates::str::contains("push_mode: direct"))
        .stdout(predicates::str::contains("direct_branch: develop"));
}

#[test]
fn profile_rename_updates_active_when_renaming_active() {
    let tmp = assert_fs::TempDir::new().unwrap();
    let user = write_user(
        &tmp,
        r#"
            active_profile = "work"
            [profiles.work.user]
            email = "e@work"
        "#,
    );
    let project = tmp.child("project");
    std::fs::create_dir_all(project.path()).unwrap();
    Command::cargo_bin("quay")
        .unwrap()
        .args([
            "--user-config",
            user.to_str().unwrap(),
            "--project",
            project.path().to_str().unwrap(),
            "profile",
            "rename",
            "work",
            "main-job",
        ])
        .assert()
        .success();
    let written = std::fs::read_to_string(&user).unwrap();
    assert!(written.contains("active_profile = \"main-job\""));
    assert!(written.contains("[profiles.main-job.user]"));
    assert!(!written.contains("[profiles.work."));
}
