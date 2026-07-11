//! Lenient discovery and metadata parsing for local skills across all four
//! mirror directories (`.agents/`, `.claude/`, `.codex/`, `.cursor/`).

use crate::config::MirrorRoot;
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

impl SkillMeta {
    /// Version string for display. Hand-written (non-frontmatter) skills have
    /// no semver — their identity is the folder content hash — so show
    /// "unversioned" instead of the meaningless `0.0.0` scanner default.
    pub fn version_display(&self) -> &str {
        match self.format {
            SkillFormat::Frontmatter => &self.version,
            SkillFormat::SlashCommand | SkillFormat::Freestyle => "unversioned",
        }
    }
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
    /// On disk only — not yet pushed to any remote and not pulled from one.
    Local,
    /// Pulled from a remote (legacy status; populated from push-log for skills
    /// that have a lockfile record from pre-0.2.0 installs).
    ///
    /// Kept for backward-compatibility display; no longer written by the scanner.
    Installed { remote: String, version: String },
    /// Same as `Installed` but the file has been modified since last pull.
    ///
    /// Kept for backward-compatibility display.
    InstalledModified { remote: String, version: String },
    /// A push-log record exists for this skill — it was pushed to a remote.
    PushedLocal {
        remote: String,
        branch: String,
        /// Empty string for direct-mode pushes (no PR was opened).
        pr_url: String,
        /// Short commit SHA from the push log; empty when log record predates Plan 9.
        commit_sha: String,
    },
}

/// One location where a skill's `SKILL.md` was found on disk.
#[derive(Debug, Clone)]
pub struct LocalLocation {
    /// Which mirror root this location belongs to.
    pub root: MirrorRoot,
    /// Absolute path to the `SKILL.md` file.
    pub path: PathBuf,
    /// SHA-256 hex digest of the file's contents.
    pub sha256: String,
}

/// One skill discovered on disk (possibly present in multiple mirror roots).
///
/// `locations` contains one entry per mirror root that has this skill; sorted
/// in canonical preference order (Agents first). The "canonical" location is
/// always `locations[0]`.
#[derive(Debug, Clone)]
pub struct LocalSkill {
    pub meta: SkillMeta,
    /// All mirror roots where this skill appears. Never empty.
    pub locations: Vec<LocalLocation>,
    pub status: ScanStatus,
}

impl LocalSkill {
    /// Convenience accessor for the canonical (first/preferred) location's path.
    pub fn canonical_path(&self) -> &Path {
        &self.locations[0].path
    }

    /// Convenience accessor for the canonical location's SHA-256.
    pub fn canonical_sha256(&self) -> &str {
        &self.locations[0].sha256
    }

    /// Returns `true` if the skill has different content across mirrors.
    pub fn has_drift(&self) -> bool {
        if self.locations.len() < 2 {
            return false;
        }
        let first = &self.locations[0].sha256;
        self.locations[1..].iter().any(|l| &l.sha256 != first)
    }

    /// Content hash of this skill over exactly the files quay pushes (SKILL.md +
    /// pushable siblings; dotfiles excluded). This is the identity signal for
    /// hand-written skills that have no semver — it matches the `content_hash`
    /// the registry writers record, so a byte-identical hub copy and this local
    /// install hash equal. Distinct from the lockfile's `folder_hash` (which
    /// includes dotfiles); see [`crate::skill_files::pushable_content_hash`].
    pub fn content_hash(&self) -> crate::error::Result<String> {
        let dir = self
            .canonical_path()
            .parent()
            .expect("SKILL.md path always has a parent dir");
        crate::skill_files::pushable_content_hash(dir)
    }
}

/// Walk all four mirror roots under `project_root`, deduplicate by folder name,
/// and return one `LocalSkill` per unique skill name (sorted alphabetically).
///
/// Skills present in multiple mirrors are folded into one `LocalSkill` with
/// multiple `locations`. The canonical location (index 0) is always the one
/// with the highest preference in [`MirrorRoot::all()`] order.
///
/// Push status is derived from `push_log` filtered to records whose
/// `project_path` matches `project_root` (or records with no `project_path`
/// for backward compatibility).
pub fn scan_local(project_root: &Path, push_log: &crate::push_log::PushLog) -> Vec<LocalSkill> {
    use std::collections::BTreeMap;

    let mut by_name: BTreeMap<String, Vec<LocalLocation>> = BTreeMap::new();
    let mut meta_by_name: BTreeMap<String, SkillMeta> = BTreeMap::new();

    for mirror in MirrorRoot::all() {
        let root = project_root.join(mirror.dir());
        let entries = match std::fs::read_dir(&root) {
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
            let folder = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            if folder.is_empty() {
                continue;
            }
            let sha256 = sha256_of(&raw);
            // Only parse metadata from the most-preferred mirror.
            if !meta_by_name.contains_key(&folder) {
                let meta = parse_skill_metadata(&raw, &skill_file);
                meta_by_name.insert(folder.clone(), meta);
            }
            by_name.entry(folder).or_default().push(LocalLocation {
                root: mirror,
                path: skill_file,
                sha256,
            });
        }
    }

    let mut out: Vec<LocalSkill> = by_name
        .into_iter()
        .filter_map(|(name, locs)| {
            let meta = meta_by_name.remove(&name)?;
            let status = derive_status(&name, push_log, project_root);
            Some(LocalSkill {
                meta,
                locations: locs,
                status,
            })
        })
        .collect();
    out.sort_by(|a, b| a.meta.name.cmp(&b.meta.name));
    out
}

/// Legacy helper: walk one level deep under each supplied `root` path.
///
/// Used by existing call sites that pass explicit root paths. New code should
/// prefer [`scan_local`] which walks all four mirror roots automatically.
///
/// Push status is derived from `push_log` without project-path filtering (all
/// records match), since this function operates on arbitrary root lists rather
/// than a single project root.
pub fn scan_local_skills(
    roots: &[PathBuf],
    push_log: &crate::push_log::PushLog,
) -> Vec<LocalSkill> {
    use std::collections::BTreeMap;

    let mut by_name: BTreeMap<String, Vec<LocalLocation>> = BTreeMap::new();
    let mut meta_by_name: BTreeMap<String, SkillMeta> = BTreeMap::new();

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
            let folder = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            if folder.is_empty() {
                continue;
            }
            let sha256 = sha256_of(&raw);
            if !meta_by_name.contains_key(&folder) {
                let meta = parse_skill_metadata(&raw, &skill_file);
                meta_by_name.insert(folder.clone(), meta);
            }
            by_name.entry(folder).or_default().push(LocalLocation {
                root: MirrorRoot::Agents,
                path: skill_file,
                sha256,
            });
        }
    }

    let mut out: Vec<LocalSkill> = by_name
        .into_iter()
        .filter_map(|(name, locs)| {
            let meta = meta_by_name.remove(&name)?;
            let status = derive_status_global(&name, push_log);
            Some(LocalSkill {
                meta,
                locations: locs,
                status,
            })
        })
        .collect();
    out.sort_by(|a, b| a.meta.name.cmp(&b.meta.name));
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

/// Derive status for a skill scanned from `project_root`, filtering push-log
/// records to those belonging to that project.
fn derive_status(
    name: &str,
    push_log: &crate::push_log::PushLog,
    project_root: &Path,
) -> ScanStatus {
    match push_log.latest_for_project(name, project_root) {
        Some(rec) => ScanStatus::PushedLocal {
            remote: rec.remote.clone(),
            branch: rec.branch.clone(),
            pr_url: rec.pr_url.clone(),
            commit_sha: rec.commit_sha.clone().unwrap_or_default(),
        },
        None => ScanStatus::Local,
    }
}

/// Derive status without project-path filtering (used by [`scan_local_skills`]
/// which operates on arbitrary root lists).
fn derive_status_global(name: &str, push_log: &crate::push_log::PushLog) -> ScanStatus {
    match push_log.latest_for(name) {
        Some(rec) => ScanStatus::PushedLocal {
            remote: rec.remote.clone(),
            branch: rec.branch.clone(),
            pr_url: rec.pr_url.clone(),
            commit_sha: rec.commit_sha.clone().unwrap_or_default(),
        },
        None => ScanStatus::Local,
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

        let log = crate::push_log::PushLog::default();
        let mut skills = scan_local_skills(std::slice::from_ref(&root), &log);
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
    fn scan_skips_directories_without_md_files() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join(".agents/skills");
        write_file(&root.join("not-a-skill/.keep"), "");
        let log = crate::push_log::PushLog::default();
        let skills = scan_local_skills(std::slice::from_ref(&root), &log);
        assert!(skills.is_empty());
    }

    #[test]
    fn scan_marks_pushed_local_when_log_has_record() {
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
            project_path: None,
        });

        let skills = scan_local_skills(std::slice::from_ref(&root), &log);
        assert!(matches!(skills[0].status, ScanStatus::PushedLocal { .. }));
    }

    #[test]
    fn scan_returns_local_status_when_no_push_log_entry() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join(".agents/skills");
        write_file(
            &root.join("foo/SKILL.md"),
            "---\nname: foo\ndescription: d\n---\n",
        );
        let log = crate::push_log::PushLog::default();
        let skills = scan_local_skills(std::slice::from_ref(&root), &log);
        assert_eq!(skills[0].status, ScanStatus::Local);
    }

    use assert_fs::prelude::*;

    #[test]
    fn version_display_hides_zero_for_non_frontmatter() {
        let mut m = SkillMeta {
            name: "x".into(),
            description: "d".into(),
            version: "0.0.0".into(),
            tags: vec![],
            format: SkillFormat::Freestyle,
        };
        assert_eq!(m.version_display(), "unversioned");
        m.format = SkillFormat::SlashCommand;
        assert_eq!(m.version_display(), "unversioned");
        m.format = SkillFormat::Frontmatter;
        m.version = "1.2.3".into();
        assert_eq!(m.version_display(), "1.2.3");
    }

    #[test]
    fn local_skill_content_hash_matches_pushable_hash() {
        let dir = assert_fs::TempDir::new().unwrap();
        dir.child("SKILL.md").write_str("# /x\nbody\n").unwrap();
        let skill = LocalSkill {
            meta: SkillMeta {
                name: "x".into(),
                description: "d".into(),
                version: "0.0.0".into(),
                tags: vec![],
                format: SkillFormat::Freestyle,
            },
            locations: vec![LocalLocation {
                root: crate::config::MirrorRoot::Agents,
                path: dir.child("SKILL.md").path().to_path_buf(),
                sha256: "irrelevant".into(),
            }],
            status: ScanStatus::Local,
        };
        let expected = crate::skill_files::pushable_content_hash(dir.path()).unwrap();
        assert_eq!(skill.content_hash().unwrap(), expected);
    }
}
