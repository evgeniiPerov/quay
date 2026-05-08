use crate::error::{QuayError, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillManifest {
    pub name: String,
    pub description: String,
    pub version: String,
    /// Free-form primary classification — whatever the hub maintainers chose, like a folder name.
    /// (e.g. "frontend", "backend", "business", "payments", "growth", or any other string.)
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub quay: QuayMeta,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuayMeta {
    #[serde(default)]
    pub min_version: Option<String>,
    #[serde(default)]
    pub resources: Vec<String>,
}

/// Parse a SKILL.md document — splits the YAML frontmatter from the body
/// and deserializes the frontmatter into a [`SkillManifest`].
pub fn parse_skill(source: &str, path_for_error: &str) -> Result<(SkillManifest, String)> {
    let trimmed = source.trim_start_matches('\u{feff}');
    let rest = trimmed
        .strip_prefix("---\n")
        .ok_or_else(|| QuayError::InvalidFrontmatter {
            path: path_for_error.into(),
            reason: "missing leading '---' frontmatter delimiter".into(),
        })?;
    let (yaml, body) = rest
        .split_once("\n---\n")
        .ok_or_else(|| QuayError::InvalidFrontmatter {
            path: path_for_error.into(),
            reason: "missing closing '---' frontmatter delimiter".into(),
        })?;
    let manifest: SkillManifest =
        serde_yaml::from_str(yaml).map_err(|e| QuayError::InvalidFrontmatter {
            path: path_for_error.into(),
            reason: e.to_string(),
        })?;
    if manifest.name.is_empty() {
        return Err(QuayError::InvalidFrontmatter {
            path: path_for_error.into(),
            reason: "field 'name' is empty".into(),
        });
    }
    if manifest.description.is_empty() {
        return Err(QuayError::InvalidFrontmatter {
            path: path_for_error.into(),
            reason: "field 'description' is empty".into(),
        });
    }
    semver::Version::parse(&manifest.version).map_err(|e| QuayError::InvalidFrontmatter {
        path: path_for_error.into(),
        reason: format!("field 'version' is not valid semver: {}", e),
    })?;
    Ok((manifest, body.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = "---\nname: csv-parse\ndescription: Parse CSV files.\nversion: 1.2.0\ncategory: backend\ntags: [data]\n---\nbody here\n";

    #[test]
    fn parses_valid_frontmatter() {
        let (m, body) = parse_skill(VALID, "test").unwrap();
        assert_eq!(m.name, "csv-parse");
        assert_eq!(m.version, "1.2.0");
        assert_eq!(m.category.as_deref(), Some("backend"));
        assert_eq!(m.tags, vec!["data".to_string()]);
        assert_eq!(body, "body here\n");
    }

    #[test]
    fn category_is_optional() {
        let no_category = "---\nname: x\ndescription: y\nversion: 1.0.0\n---\nbody\n";
        let (m, _) = parse_skill(no_category, "test").unwrap();
        assert_eq!(m.category, None);
    }

    #[test]
    fn rejects_missing_open_delimiter() {
        let bad = "name: foo\n";
        let err = parse_skill(bad, "test").unwrap_err();
        assert!(matches!(err, QuayError::InvalidFrontmatter { .. }));
    }

    #[test]
    fn rejects_missing_close_delimiter() {
        let bad = "---\nname: foo\nversion: 1.0.0\nbody\n";
        let err = parse_skill(bad, "test").unwrap_err();
        assert!(format!("{}", err).contains("closing"));
    }

    #[test]
    fn rejects_missing_required_field() {
        let bad = "---\nname: foo\nversion: 1.0.0\n---\nbody\n";
        let err = parse_skill(bad, "test").unwrap_err();
        assert!(format!("{}", err).contains("description"));
    }

    #[test]
    fn rejects_invalid_semver() {
        let bad = "---\nname: foo\ndescription: x\nversion: not-a-version\n---\nbody\n";
        let err = parse_skill(bad, "test").unwrap_err();
        assert!(format!("{}", err).contains("semver"));
    }
}
