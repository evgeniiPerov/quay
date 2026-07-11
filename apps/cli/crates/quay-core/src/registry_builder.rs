//! Build a [`Registry`] from a hub clone's `skills/` directory on disk.
//!
//! Used by the pusher (single-skill update) and `quay rebuild-registry`
//! (full regeneration). The output mirrors what the spec's hub-side CI
//! would produce: one entry per `skills/<name>/SKILL.md` discovered.

use crate::error::{QuayError, Result};
use crate::registry::{Registry, RegistryEntry};
use crate::scanner::parse_skill_metadata;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::Path;

/// Walk `<hub_clone>/skills/<name>/SKILL.md` and return a [`Registry`]
/// describing every skill found. Missing `skills/` directory yields an
/// empty registry. The `hub` field is set to `hub_name`.
pub fn build_from_hub_clone(hub_clone: &Path, hub_name: &str) -> Result<Registry> {
    let mut registry = Registry {
        hub: hub_name.to_string(),
        generated_at: chrono::Utc::now().to_rfc3339(),
        schema_version: 1,
        skills: BTreeMap::new(),
    };

    let skills_dir = hub_clone.join("skills");
    if !skills_dir.is_dir() {
        return Ok(registry);
    }

    let entries = std::fs::read_dir(&skills_dir).map_err(|source| QuayError::Io {
        path: skills_dir.display().to_string(),
        source,
    })?;
    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let name = match dir.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        let skill_md = dir.join("SKILL.md");
        if !skill_md.is_file() {
            continue;
        }

        let raw_bytes = std::fs::read(&skill_md).map_err(|source| QuayError::Io {
            path: skill_md.display().to_string(),
            source,
        })?;
        let raw = String::from_utf8_lossy(&raw_bytes);
        let meta = parse_skill_metadata(&raw, &skill_md);

        let mut hasher = Sha256::new();
        hasher.update(&raw_bytes);
        let sha = hex::encode(hasher.finalize());

        let files = crate::skill_files::collect_skill_files(&dir)?;
        let content_hash = crate::lock_hash::folder_hash(&dir)?;

        registry.skills.insert(
            name.clone(),
            RegistryEntry {
                version: meta.version.clone(),
                description: meta.description.clone(),
                category: None,
                tags: meta.tags.clone(),
                path: format!("skills/{}", name),
                sha,
                files,
                source_format: meta.format,
                content_hash,
            },
        );
    }

    Ok(registry)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn build_indexes_all_skills_in_skills_dir() {
        let tmp = TempDir::new().unwrap();
        let hub = tmp.path();
        fs::create_dir_all(hub.join("skills/foo")).unwrap();
        fs::write(
            hub.join("skills/foo/SKILL.md"),
            "---\nname: foo\ndescription: f\nversion: 0.1.0\n---\n",
        )
        .unwrap();
        fs::create_dir_all(hub.join("skills/bar")).unwrap();
        fs::write(hub.join("skills/bar/SKILL.md"), "# /bar\n\nbar skill\n").unwrap();

        let r = build_from_hub_clone(hub, "test-hub").unwrap();
        assert_eq!(r.hub, "test-hub");
        assert_eq!(r.skills.len(), 2);
        assert!(r.skills.contains_key("foo"));
        assert!(r.skills.contains_key("bar"));
        assert_eq!(r.skills["foo"].path, "skills/foo");
        assert_eq!(r.skills["foo"].files, vec!["SKILL.md".to_string()]);
        assert_eq!(
            r.skills["bar"].source_format,
            crate::scanner::SkillFormat::SlashCommand
        );
    }

    #[test]
    fn build_sets_content_hash_to_folder_hash() {
        let tmp = TempDir::new().unwrap();
        let hub = tmp.path();
        fs::create_dir_all(hub.join("skills/foo")).unwrap();
        fs::write(hub.join("skills/foo/SKILL.md"), "# /foo\nbody\n").unwrap();
        fs::write(hub.join("skills/foo/helper.py"), "print('x')\n").unwrap();

        let reg = build_from_hub_clone(hub, "h").unwrap();
        let entry = reg.entry("foo").expect("foo indexed");

        let expected = crate::lock_hash::folder_hash(&hub.join("skills/foo")).unwrap();
        assert_eq!(entry.content_hash, expected);
        assert!(!entry.content_hash.is_empty());
    }

    #[test]
    fn missing_skills_dir_yields_empty_registry() {
        let tmp = TempDir::new().unwrap();
        let r = build_from_hub_clone(tmp.path(), "h").unwrap();
        assert!(r.skills.is_empty());
        assert_eq!(r.hub, "h");
    }

    #[test]
    fn includes_extra_files_alongside_skill_md() {
        let tmp = TempDir::new().unwrap();
        let hub = tmp.path();
        fs::create_dir_all(hub.join("skills/foo")).unwrap();
        fs::write(
            hub.join("skills/foo/SKILL.md"),
            "---\nname: foo\ndescription: f\nversion: 0.1.0\n---\n",
        )
        .unwrap();
        fs::write(hub.join("skills/foo/extra.txt"), "x").unwrap();
        let r = build_from_hub_clone(hub, "h").unwrap();
        assert_eq!(
            r.skills["foo"].files,
            vec!["SKILL.md".to_string(), "extra.txt".to_string()]
        );
    }

    #[test]
    fn includes_nested_subdir_files() {
        let tmp = TempDir::new().unwrap();
        let hub = tmp.path();
        fs::create_dir_all(hub.join("skills/foo/scripts")).unwrap();
        fs::create_dir_all(hub.join("skills/foo/agents")).unwrap();
        fs::write(
            hub.join("skills/foo/SKILL.md"),
            "---\nname: foo\ndescription: f\nversion: 0.1.0\n---\n",
        )
        .unwrap();
        fs::write(hub.join("skills/foo/scripts/sync.mjs"), "code").unwrap();
        fs::write(hub.join("skills/foo/agents/openai.yaml"), "cfg").unwrap();

        let r = build_from_hub_clone(hub, "h").unwrap();
        assert_eq!(
            r.skills["foo"].files,
            vec![
                "SKILL.md".to_string(),
                "agents/openai.yaml".to_string(),
                "scripts/sync.mjs".to_string(),
            ]
        );
    }
}
