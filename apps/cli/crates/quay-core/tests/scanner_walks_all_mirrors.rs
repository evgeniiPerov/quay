//! Integration test: `scan_local` walks all four mirror roots and deduplicates
//! by folder name so a skill present in two mirrors appears as one `LocalSkill`
//! with two `locations`.

use quay_core::scanner::scan_local;
use quay_core::MirrorRoot;
use std::fs;
use std::path::Path;

fn write_file(path: &Path, body: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
}

#[test]
fn scan_finds_skill_in_single_mirror() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path();
    let body = "---\nname: foo\ndescription: A skill\nversion: 1.0.0\n---\nbody\n";
    write_file(&root.join(".agents/skills/foo/SKILL.md"), body);

    let log = quay_core::push_log::PushLog::default();
    let skills = scan_local(root, &log);

    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].meta.name, "foo");
    assert_eq!(skills[0].locations.len(), 1);
    assert_eq!(skills[0].locations[0].root, MirrorRoot::Agents);
}

#[test]
fn scan_deduplicates_skill_across_two_mirrors() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path();
    let body = "---\nname: foo\ndescription: A skill\nversion: 1.0.0\n---\nbody\n";
    // Same content in both .agents and .claude
    write_file(&root.join(".agents/skills/foo/SKILL.md"), body);
    write_file(&root.join(".claude/skills/foo/SKILL.md"), body);

    let log = quay_core::push_log::PushLog::default();
    let skills = scan_local(root, &log);

    assert_eq!(skills.len(), 1, "must deduplicate into one skill");
    assert_eq!(skills[0].meta.name, "foo");
    assert_eq!(skills[0].locations.len(), 2);
    // Agents comes first (canonical preference order)
    assert_eq!(skills[0].locations[0].root, MirrorRoot::Agents);
    assert_eq!(skills[0].locations[1].root, MirrorRoot::Claude);
}

#[test]
fn scan_finds_skills_across_all_four_mirrors() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path();
    let body = "---\nname: multi\ndescription: Multi-mirror skill\nversion: 0.1.0\n---\nbody\n";

    write_file(&root.join(".agents/skills/multi/SKILL.md"), body);
    write_file(&root.join(".claude/skills/multi/SKILL.md"), body);
    write_file(&root.join(".codex/skills/multi/SKILL.md"), body);
    write_file(&root.join(".cursor/skills/multi/SKILL.md"), body);

    let log = quay_core::push_log::PushLog::default();
    let skills = scan_local(root, &log);

    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].locations.len(), 4);
    // Canonical preference: Agents > Claude > Codex > Cursor
    assert_eq!(skills[0].locations[0].root, MirrorRoot::Agents);
    assert_eq!(skills[0].locations[1].root, MirrorRoot::Claude);
    assert_eq!(skills[0].locations[2].root, MirrorRoot::Codex);
    assert_eq!(skills[0].locations[3].root, MirrorRoot::Cursor);
}

#[test]
fn scan_returns_distinct_skills_from_different_names() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path();
    write_file(
        &root.join(".agents/skills/alpha/SKILL.md"),
        "---\nname: alpha\ndescription: a\nversion: 0.1.0\n---\nbody\n",
    );
    write_file(
        &root.join(".claude/skills/beta/SKILL.md"),
        "---\nname: beta\ndescription: b\nversion: 0.1.0\n---\nbody\n",
    );

    let log = quay_core::push_log::PushLog::default();
    let skills = scan_local(root, &log);

    assert_eq!(skills.len(), 2);
    let names: Vec<&str> = skills.iter().map(|s| s.meta.name.as_str()).collect();
    assert!(names.contains(&"alpha"));
    assert!(names.contains(&"beta"));
}

#[test]
fn scan_returns_alphabetically_sorted_skills() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path();
    write_file(
        &root.join(".agents/skills/zebra/SKILL.md"),
        "---\nname: zebra\ndescription: z\nversion: 0.1.0\n---\n",
    );
    write_file(
        &root.join(".agents/skills/apple/SKILL.md"),
        "---\nname: apple\ndescription: a\nversion: 0.1.0\n---\n",
    );

    let log = quay_core::push_log::PushLog::default();
    let skills = scan_local(root, &log);

    assert_eq!(skills[0].meta.name, "apple");
    assert_eq!(skills[1].meta.name, "zebra");
}

#[test]
fn scan_ignores_missing_mirror_roots() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path();
    // Only .agents exists; .claude/.codex/.cursor are absent
    write_file(
        &root.join(".agents/skills/foo/SKILL.md"),
        "---\nname: foo\ndescription: f\nversion: 0.1.0\n---\nbody\n",
    );

    let log = quay_core::push_log::PushLog::default();
    let skills = scan_local(root, &log);

    // Should not panic and should return the one skill
    assert_eq!(skills.len(), 1);
}
