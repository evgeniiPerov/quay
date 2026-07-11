use crate::error::{QuayError, Result};
use crate::scanner::SkillFormat;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Registry {
    pub hub: String,
    pub generated_at: String,
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    pub skills: BTreeMap<String, RegistryEntry>,
}

fn default_schema_version() -> u32 {
    1
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryEntry {
    pub version: String,
    pub description: String,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub path: String,
    pub sha: String,
    pub files: Vec<String>,
    /// New in Plan 8. Defaults to `Frontmatter` when reading old registry.json.
    #[serde(default = "default_entry_source_format")]
    pub source_format: SkillFormat,
    /// New in the content-hash feature. Content hash of the skill's pushable
    /// file set (`skill_files::pushable_content_hash`) — dotfiles, dotdirs and
    /// symlinks excluded, so it matches `LocalSkill::content_hash` for a
    /// byte-identical install. Empty only when read from a registry.json that
    /// predates this field.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub content_hash: String,
}

fn default_entry_source_format() -> SkillFormat {
    SkillFormat::Frontmatter
}

impl Registry {
    pub fn parse(json: &str) -> Result<Self> {
        serde_json::from_str(json).map_err(|e| QuayError::InvalidRegistry {
            reason: e.to_string(),
        })
    }

    pub fn entry(&self, name: &str) -> Option<&RegistryEntry> {
        self.skills.get(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
        "hub": "my-org-skills",
        "generated_at": "2026-05-08T10:39:00Z",
        "schema_version": 1,
        "skills": {
            "csv-parse": {
                "version": "1.2.0",
                "description": "Parse CSV with auto-delimiter detection",
                "tags": ["data", "parsing", "backend"],
                "category": "backend",
                "path": "skills/csv-parse",
                "sha": "abc123def",
                "files": ["SKILL.md", "resources/delimiters.md"]
            }
        }
    }"#;

    #[test]
    fn parses_sample() {
        let r = Registry::parse(SAMPLE).unwrap();
        assert_eq!(r.hub, "my-org-skills");
        assert_eq!(r.schema_version, 1);
        let e = r.entry("csv-parse").unwrap();
        assert_eq!(e.version, "1.2.0");
        assert_eq!(e.files.len(), 2);
        assert_eq!(e.tags, vec!["data", "parsing", "backend"]);
        assert_eq!(e.category.as_deref(), Some("backend"));
    }

    #[test]
    fn missing_skill_returns_none() {
        let r = Registry::parse(SAMPLE).unwrap();
        assert!(r.entry("does-not-exist").is_none());
    }

    #[test]
    fn rejects_malformed_json() {
        let err = Registry::parse("{ not json").unwrap_err();
        assert!(matches!(err, QuayError::InvalidRegistry { .. }));
    }

    #[test]
    fn schema_version_defaults_to_1() {
        let no_version = r#"{
            "hub": "x",
            "generated_at": "2026-05-08T10:39:00Z",
            "skills": {}
        }"#;
        let r = Registry::parse(no_version).unwrap();
        assert_eq!(r.schema_version, 1);
    }

    #[test]
    fn category_and_tags_default_when_omitted() {
        let minimal = r#"{
            "hub": "x",
            "generated_at": "2026-05-08T10:39:00Z",
            "skills": {
                "x": {
                    "version": "0.1.0",
                    "description": "y",
                    "path": "skills/x",
                    "sha": "0",
                    "files": ["SKILL.md"]
                }
            }
        }"#;
        let r = Registry::parse(minimal).unwrap();
        let e = r.entry("x").unwrap();
        assert_eq!(e.category, None);
        assert!(e.tags.is_empty());
    }

    #[test]
    fn registry_entry_reads_old_json_without_source_format_as_frontmatter() {
        let json = r#"{
            "hub": "old-hub",
            "generated_at": "2026-05-08T10:39:00Z",
            "skills": {
                "foo": {
                    "version": "1.0.0",
                    "description": "old",
                    "path": "skills/foo",
                    "sha": "deadbeef",
                    "files": ["SKILL.md"]
                }
            }
        }"#;
        let registry = Registry::parse(json).unwrap();
        let entry = registry.entry("foo").unwrap();
        assert_eq!(
            entry.source_format,
            crate::scanner::SkillFormat::Frontmatter
        );
    }

    #[test]
    fn parses_entry_without_content_hash_as_empty() {
        // Old registry.json (pre-feature) has no content_hash key.
        let reg = Registry::parse(SAMPLE).unwrap();
        let entry = reg.entry("csv-parse").expect("csv-parse present");
        assert_eq!(entry.content_hash, "");
    }

    #[test]
    fn registry_entry_round_trips_explicit_source_format() {
        let json = r#"{
            "hub": "new-hub",
            "generated_at": "2026-05-09T10:39:00Z",
            "skills": {
                "foo": {
                    "version": "1.0.0",
                    "description": "new",
                    "path": "skills/foo",
                    "sha": "deadbeef",
                    "files": ["SKILL.md"],
                    "source_format": "slash_command"
                }
            }
        }"#;
        let registry = Registry::parse(json).unwrap();
        let entry = registry.entry("foo").unwrap();
        assert_eq!(
            entry.source_format,
            crate::scanner::SkillFormat::SlashCommand
        );
    }
}
