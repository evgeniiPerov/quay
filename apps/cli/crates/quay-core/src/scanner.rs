//! Lenient discovery and metadata parsing for local skills in `.agents/skills/`.

use crate::lockfile::Lockfile;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Detected source format of a SKILL.md (or `.md`) file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillFormat {
    /// File begins with a `---\n…\n---` YAML frontmatter block.
    Frontmatter,
    /// First non-blank line is `# /<name>` (Claude slash-command style).
    SlashCommand,
    /// Anything else — markdown without recognised metadata.
    Freestyle,
}

/// Lenient metadata derived from a skill file.
///
/// Required fields (`name`, `description`, `version`) always have values —
/// the parser fills in defaults from directory name / first paragraph / `"0.0.0"`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillMeta {
    pub name: String,
    pub description: String,
    pub version: String,
    pub tags: Vec<String>,
    pub format: SkillFormat,
}

/// Parse what metadata can be derived from a raw skill file.
///
/// `path` is used for the directory-name fallback and never read from disk here.
pub fn parse_skill_metadata(raw: &str, path: &Path) -> SkillMeta {
    let trimmed = raw.trim_start_matches('\u{feff}');

    // Frontmatter branch.
    if let Some(rest) = trimmed.strip_prefix("---\n") {
        if let Some((yaml, _body)) = rest.split_once("\n---\n") {
            #[derive(Deserialize)]
            struct Front {
                #[serde(default)]
                name: Option<String>,
                #[serde(default)]
                description: Option<String>,
                #[serde(default)]
                version: Option<String>,
                #[serde(default)]
                tags: Vec<String>,
            }
            let front: Front = serde_yaml::from_str(yaml).unwrap_or(Front {
                name: None,
                description: None,
                version: None,
                tags: Vec::new(),
            });
            let dir_name = dir_name_from_path(path);
            return SkillMeta {
                name: front.name.unwrap_or_else(|| dir_name.clone()),
                description: front.description.unwrap_or_default(),
                version: front.version.unwrap_or_else(|| "0.0.0".to_string()),
                tags: front.tags,
                format: SkillFormat::Frontmatter,
            };
        }
    }

    // Slash-command branch: first non-blank line is "# /<name>".
    let first_non_blank = trimmed.lines().find(|l| !l.trim().is_empty());
    if let Some(first) = first_non_blank {
        if let Some(name_part) = first.strip_prefix("# /") {
            let name: String = name_part
                .chars()
                .take_while(|c| !c.is_whitespace() && *c != '[')
                .collect();
            let desc = trimmed
                .lines()
                .skip_while(|l| l.trim().is_empty() || l.starts_with("# /"))
                .take_while(|l| !l.trim().is_empty() && !l.starts_with('#'))
                .collect::<Vec<_>>()
                .join(" ")
                .trim()
                .to_string();
            return SkillMeta {
                name: if name.is_empty() {
                    dir_name_from_path(path)
                } else {
                    name
                },
                description: desc,
                version: "0.0.0".to_string(),
                tags: Vec::new(),
                format: SkillFormat::SlashCommand,
            };
        }
    }

    // Freestyle fallback.
    let desc = trimmed
        .lines()
        .skip_while(|l| l.trim().is_empty() || l.starts_with('#'))
        .take_while(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string();
    SkillMeta {
        name: dir_name_from_path(path),
        description: desc,
        version: "0.0.0".to_string(),
        tags: Vec::new(),
        format: SkillFormat::Freestyle,
    }
}

/// Sync state of a discovered local skill.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScanStatus {
    /// On disk; no lockfile entry.
    Local,
    /// On disk and in lockfile; primary-file hash matches.
    Installed { remote: String, version: String },
    /// On disk and in lockfile; primary-file hash differs.
    InstalledModified { remote: String, version: String },
    /// On disk; no lockfile entry, but a recent push-log record exists.
    PushedLocal {
        remote: String,
        branch: String,
        /// Empty string for direct-mode pushes (no PR was opened).
        pr_url: String,
        /// Short commit SHA from the push log; empty when log record predates Plan 9.
        commit_sha: String,
    },
}

/// One skill discovered on disk.
#[derive(Debug, Clone)]
pub struct LocalSkill {
    pub meta: SkillMeta,
    pub path: PathBuf,
    pub sha256: String,
    pub status: ScanStatus,
}

/// Walk one level deep under each `root` and return one `LocalSkill` per
/// directory containing a markdown skill file.
pub fn scan_local_skills(
    roots: &[PathBuf],
    lockfile: &Lockfile,
    push_log: &crate::push_log::PushLog,
) -> Vec<LocalSkill> {
    let mut out = Vec::new();
    for root in roots {
        let entries = match std::fs::read_dir(root) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Some(skill_file) = pick_skill_file(&path) else {
                continue;
            };
            let raw = match std::fs::read_to_string(&skill_file) {
                Ok(s) => s,
                Err(_) => continue,
            };
            let meta = parse_skill_metadata(&raw, &skill_file);
            let sha256 = sha256_of(&raw);
            let status = derive_status(&meta.name, &sha256, lockfile, push_log);
            out.push(LocalSkill {
                meta,
                path: skill_file,
                sha256,
                status,
            });
        }
    }
    out
}

fn pick_skill_file(dir: &Path) -> Option<PathBuf> {
    let skill_md = dir.join("SKILL.md");
    if skill_md.is_file() {
        return Some(skill_md);
    }
    let mut mds: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().map(|x| x == "md").unwrap_or(false))
        .collect();
    if mds.is_empty() {
        return None;
    }
    mds.sort();
    Some(mds.remove(0))
}

fn sha256_of(s: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(s.as_bytes());
    hex::encode(h.finalize())
}

fn derive_status(
    name: &str,
    on_disk_hash: &str,
    lockfile: &Lockfile,
    push_log: &crate::push_log::PushLog,
) -> ScanStatus {
    match (
        lockfile.skill_primary_sha(name),
        lockfile.skill_remote_version(name),
    ) {
        (Some(locked_hash), Some((remote, version))) if locked_hash == on_disk_hash => {
            ScanStatus::Installed {
                remote: remote.to_string(),
                version: version.to_string(),
            }
        }
        (Some(_), Some((remote, version))) => ScanStatus::InstalledModified {
            remote: remote.to_string(),
            version: version.to_string(),
        },
        (None, _) => match push_log.latest_for(name) {
            Some(rec) => ScanStatus::PushedLocal {
                remote: rec.remote.clone(),
                branch: rec.branch.clone(),
                pr_url: rec.pr_url.clone(),
                commit_sha: rec.commit_sha.clone().unwrap_or_default(),
            },
            None => ScanStatus::Local,
        },
        // Degenerate lockfile: sha present but no remote/version recorded.
        (Some(_), None) => ScanStatus::Local,
    }
}

fn dir_name_from_path(path: &Path) -> String {
    path.parent()
        .and_then(|p| p.file_name())
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| {
            path.file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default()
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn frontmatter_skill_is_detected() {
        let raw = "---\nname: foo\ndescription: A foo skill\nversion: 1.2.3\ntags: [a, b]\n---\n# Foo body\n";
        let path = PathBuf::from("/tmp/skills/foo/SKILL.md");
        let meta = parse_skill_metadata(raw, &path);
        assert_eq!(meta.format, SkillFormat::Frontmatter);
        assert_eq!(meta.name, "foo");
        assert_eq!(meta.description, "A foo skill");
        assert_eq!(meta.version, "1.2.3");
        assert_eq!(meta.tags, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn frontmatter_with_missing_optional_fields_uses_defaults() {
        let raw = "---\nname: foo\ndescription: hi\n---\nbody\n";
        let path = PathBuf::from("/tmp/skills/foo/SKILL.md");
        let meta = parse_skill_metadata(raw, &path);
        assert_eq!(meta.version, "0.0.0");
        assert!(meta.tags.is_empty());
    }

    #[test]
    fn slash_command_skill_is_detected() {
        let raw =
            "# /add-entity [name]\n\nScaffold a MikroORM entity.\n\n## Instructions\n\n1. Foo.\n";
        let path = PathBuf::from("/tmp/skills/add-entity/SKILL.md");
        let meta = parse_skill_metadata(raw, &path);
        assert_eq!(meta.format, SkillFormat::SlashCommand);
        assert_eq!(meta.name, "add-entity");
        assert_eq!(meta.description, "Scaffold a MikroORM entity.");
        assert_eq!(meta.version, "0.0.0");
    }

    #[test]
    fn slash_command_with_no_following_paragraph_uses_empty_description() {
        let raw = "# /lone-cmd\n";
        let path = PathBuf::from("/tmp/skills/lone-cmd/SKILL.md");
        let meta = parse_skill_metadata(raw, &path);
        assert_eq!(meta.format, SkillFormat::SlashCommand);
        assert_eq!(meta.name, "lone-cmd");
        assert_eq!(meta.description, "");
    }

    #[test]
    fn freestyle_markdown_uses_dir_name_and_first_paragraph() {
        let raw = "## Notes\n\nSome free-form notes about the skill.\n";
        let path = PathBuf::from("/tmp/skills/random-stuff/SKILL.md");
        let meta = parse_skill_metadata(raw, &path);
        assert_eq!(meta.format, SkillFormat::Freestyle);
        assert_eq!(meta.name, "random-stuff");
        assert_eq!(meta.description, "Some free-form notes about the skill.");
    }

    #[test]
    fn empty_file_falls_back_to_dir_name() {
        let raw = "";
        let path = PathBuf::from("/tmp/skills/blank/SKILL.md");
        let meta = parse_skill_metadata(raw, &path);
        assert_eq!(meta.format, SkillFormat::Freestyle);
        assert_eq!(meta.name, "blank");
        assert_eq!(meta.description, "");
    }

    // --- scan_local_skills tests ---

    use std::fs;
    use tempfile::TempDir;

    fn write_file(path: &Path, body: &str) {
        if let Some(p) = path.parent() {
            fs::create_dir_all(p).unwrap();
        }
        fs::write(path, body).unwrap();
    }

    fn sha256_hex(s: &str) -> String {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(s.as_bytes());
        hex::encode(h.finalize())
    }

    #[test]
    fn scan_finds_three_format_variants_as_local() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join(".agents/skills");
        write_file(
            &root.join("a-front/SKILL.md"),
            "---\nname: a-front\ndescription: front\n---\nbody\n",
        );
        write_file(
            &root.join("b-slash/SKILL.md"),
            "# /b-slash\n\nA slash skill.\n",
        );
        write_file(&root.join("c-free/SKILL.md"), "Just markdown.\n");

        let lock = Lockfile::default();
        let log = crate::push_log::PushLog::default();
        let mut skills = scan_local_skills(std::slice::from_ref(&root), &lock, &log);
        skills.sort_by(|a, b| a.meta.name.cmp(&b.meta.name));

        assert_eq!(skills.len(), 3);
        assert_eq!(skills[0].meta.name, "a-front");
        assert_eq!(skills[0].meta.format, SkillFormat::Frontmatter);
        assert_eq!(skills[0].status, ScanStatus::Local);

        assert_eq!(skills[1].meta.name, "b-slash");
        assert_eq!(skills[1].meta.format, SkillFormat::SlashCommand);
        assert_eq!(skills[1].status, ScanStatus::Local);

        assert_eq!(skills[2].meta.format, SkillFormat::Freestyle);
    }

    #[test]
    fn scan_marks_installed_when_lockfile_hash_matches() {
        use crate::lockfile::{LockedFile, LockedSkill};

        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join(".agents/skills");
        let body = "---\nname: dep\ndescription: d\n---\nbody\n";
        write_file(&root.join("dep/SKILL.md"), body);

        let mut lock = Lockfile::default();
        lock.skills.insert(
            "dep".into(),
            LockedSkill {
                remote: "hub".into(),
                version: "1.0.0".into(),
                sha: "irrelevant".into(),
                path: "skills/dep".into(),
                files: vec![LockedFile {
                    path: "skills/dep/SKILL.md".into(),
                    sha256: sha256_hex(body),
                }],
                installed_at: "2026-05-09T00:00:00Z".into(),
            },
        );

        let log = crate::push_log::PushLog::default();
        let skills = scan_local_skills(std::slice::from_ref(&root), &lock, &log);
        assert_eq!(skills.len(), 1);
        assert_eq!(
            skills[0].status,
            ScanStatus::Installed {
                remote: "hub".into(),
                version: "1.0.0".into()
            }
        );
    }

    #[test]
    fn scan_marks_installed_modified_when_hash_differs() {
        use crate::lockfile::{LockedFile, LockedSkill};

        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join(".agents/skills");
        let on_disk = "---\nname: dep\ndescription: edited\n---\nbody\n";
        write_file(&root.join("dep/SKILL.md"), on_disk);

        let mut lock = Lockfile::default();
        lock.skills.insert(
            "dep".into(),
            LockedSkill {
                remote: "hub".into(),
                version: "1.0.0".into(),
                sha: "irrelevant".into(),
                path: "skills/dep".into(),
                files: vec![LockedFile {
                    path: "skills/dep/SKILL.md".into(),
                    sha256: sha256_hex("---\nname: dep\ndescription: original\n---\nbody\n"),
                }],
                installed_at: "2026-05-09T00:00:00Z".into(),
            },
        );

        let log = crate::push_log::PushLog::default();
        let skills = scan_local_skills(std::slice::from_ref(&root), &lock, &log);
        assert!(matches!(
            skills[0].status,
            ScanStatus::InstalledModified { .. }
        ));
    }

    #[test]
    fn scan_skips_directories_without_md_files() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join(".agents/skills");
        write_file(&root.join("not-a-skill/.keep"), "");
        let lock = Lockfile::default();
        let log = crate::push_log::PushLog::default();
        let skills = scan_local_skills(std::slice::from_ref(&root), &lock, &log);
        assert!(skills.is_empty());
    }

    #[test]
    fn scan_marks_pushed_local_when_log_has_record_and_no_lockfile_entry() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join(".agents/skills");
        write_file(
            &root.join("nu/SKILL.md"),
            "---\nname: nu\ndescription: x\n---\n",
        );

        let mut log = crate::push_log::PushLog::default();
        log.records.push(crate::push_log::PushRecord {
            name: "nu".into(),
            remote: "hub".into(),
            branch: "quay/nu-0.0.0".into(),
            pr_url: "https://example/pr/9".into(),
            pushed_at: "2026-05-09T18:30:00Z".into(),
            commit_sha: None,
        });

        let lock = Lockfile::default();
        let skills = scan_local_skills(std::slice::from_ref(&root), &lock, &log);
        assert!(matches!(skills[0].status, ScanStatus::PushedLocal { .. }));
    }
}
