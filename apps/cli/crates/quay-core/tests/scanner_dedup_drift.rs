//! Integration test: drift detection when the same skill has different content
//! in different mirror roots.

use quay_core::scanner::scan_local;
use quay_core::MirrorRoot;
use std::fs;
use std::path::Path;

fn write_file(path: &Path, body: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
}

#[test]
fn no_drift_when_single_location() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path();
    write_file(
        &root.join(".agents/skills/foo/SKILL.md"),
        "---\nname: foo\ndescription: original\nversion: 1.0.0\n---\nbody\n",
    );

    let log = quay_core::push_log::PushLog::default();
    let skills = scan_local(root, &log);

    assert_eq!(skills.len(), 1);
    assert!(!skills[0].has_drift(), "single location cannot drift");
}

#[test]
fn no_drift_when_both_mirrors_identical() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path();
    let body = "---\nname: foo\ndescription: same\nversion: 1.0.0\n---\nbody\n";
    write_file(&root.join(".agents/skills/foo/SKILL.md"), body);
    write_file(&root.join(".claude/skills/foo/SKILL.md"), body);

    let log = quay_core::push_log::PushLog::default();
    let skills = scan_local(root, &log);

    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].locations.len(), 2);
    assert!(!skills[0].has_drift(), "identical content should not drift");
    // Both shas must be equal
    assert_eq!(skills[0].locations[0].sha256, skills[0].locations[1].sha256);
}

#[test]
fn drift_detected_when_content_differs_across_mirrors() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path();
    let body_agents = "---\nname: foo\ndescription: version A\nversion: 1.0.0\n---\nbody\n";
    let body_claude = "---\nname: foo\ndescription: version B\nversion: 1.1.0\n---\nother body\n";
    write_file(&root.join(".agents/skills/foo/SKILL.md"), body_agents);
    write_file(&root.join(".claude/skills/foo/SKILL.md"), body_claude);

    let log = quay_core::push_log::PushLog::default();
    let skills = scan_local(root, &log);

    assert_eq!(skills.len(), 1, "must deduplicate");
    assert_eq!(skills[0].locations.len(), 2);
    // Agents is canonical (index 0)
    assert_eq!(skills[0].locations[0].root, MirrorRoot::Agents);
    assert_eq!(skills[0].locations[1].root, MirrorRoot::Claude);
    // SHAs must differ
    assert_ne!(
        skills[0].locations[0].sha256, skills[0].locations[1].sha256,
        "different content must produce different sha256"
    );
    assert!(skills[0].has_drift(), "has_drift() must return true");
}

#[test]
fn drift_detected_across_three_mirrors() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path();

    write_file(
        &root.join(".agents/skills/tool/SKILL.md"),
        "---\nname: tool\ndescription: v1\nversion: 1.0.0\n---\n",
    );
    write_file(
        &root.join(".claude/skills/tool/SKILL.md"),
        "---\nname: tool\ndescription: v2\nversion: 2.0.0\n---\n",
    );
    write_file(
        &root.join(".codex/skills/tool/SKILL.md"),
        "---\nname: tool\ndescription: v1\nversion: 1.0.0\n---\n", // same as agents
    );

    let log = quay_core::push_log::PushLog::default();
    let skills = scan_local(root, &log);

    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].locations.len(), 3);
    assert!(skills[0].has_drift(), "claude differs from agents");
    // Agents and Codex should match
    assert_eq!(skills[0].locations[0].sha256, skills[0].locations[2].sha256);
}

#[test]
fn canonical_path_points_to_agents_root() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path();
    let body = "---\nname: my-tool\ndescription: t\nversion: 0.1.0\n---\n";
    write_file(&root.join(".agents/skills/my-tool/SKILL.md"), body);
    write_file(&root.join(".claude/skills/my-tool/SKILL.md"), body);

    let log = quay_core::push_log::PushLog::default();
    let skills = scan_local(root, &log);

    let canonical = skills[0].canonical_path();
    // Separators normalized: MirrorRoot::dir() is the literal ".agents/skills",
    // so joining it on Windows yields a mixed path like `...\.agents/skills\my-tool`.
    let as_posix = canonical.to_str().unwrap().replace('\\', "/");
    assert!(
        as_posix.contains(".agents/skills/my-tool"),
        "canonical path should be under .agents/skills/, got: {}",
        canonical.display()
    );
}
