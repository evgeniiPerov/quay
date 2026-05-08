//! Implementation of `quay validate <skill>`.

use quay_core::{parse_skill, QuayError};
use serde_json::json;
use std::path::Path;

/// All information produced by a successful [`validate_skill`] call.
#[derive(Debug)]
pub struct ValidateOutcome {
    /// The skill name as supplied.
    pub skill: String,
    /// Whether the frontmatter parsed cleanly and all required fields are present.
    pub frontmatter_ok: bool,
    /// Semver version string from the frontmatter; `None` when validation failed.
    pub version: Option<String>,
    /// Validation error messages. Empty means the skill is valid.
    pub errors: Vec<String>,
}

/// Validate a locally installed skill's `SKILL.md` without any output side-effects.
///
/// Returns `Ok(ValidateOutcome)` in all cases where the file can be read;
/// `Err` only when the file cannot be found or opened (I/O failure).
pub fn validate_skill(skill: &str, project: &Path) -> Result<ValidateOutcome, QuayError> {
    let path = project.join(".agents/skills").join(skill).join("SKILL.md");
    if !path.exists() {
        return Err(QuayError::SkillNotFound {
            name: skill.to_string(),
            remote: "local".into(),
        });
    }
    let text = std::fs::read_to_string(&path).map_err(|e| QuayError::Io {
        path: path.display().to_string(),
        source: e,
    })?;

    let mut errors = Vec::new();
    let (frontmatter_ok, version) = match parse_skill(&text, &path.display().to_string()) {
        Ok((manifest, _body)) => (true, Some(manifest.version)),
        Err(e) => {
            errors.push(e.to_string());
            (false, None)
        }
    };

    Ok(ValidateOutcome {
        skill: skill.to_string(),
        frontmatter_ok,
        version,
        errors,
    })
}

/// Validate the frontmatter of a locally installed skill, offline.
pub fn run(skill: &str, project: &Path, json: bool) -> Result<(), Box<dyn std::error::Error>> {
    let outcome = validate_skill(skill, project)?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "skill": outcome.skill,
                "ok": outcome.errors.is_empty(),
                "errors": outcome.errors,
            }))?
        );
    } else if outcome.errors.is_empty() {
        println!(
            "ok: {} v{}",
            outcome.skill,
            outcome.version.as_deref().unwrap_or("unknown")
        );
    } else {
        eprintln!("{} has {} error(s):", outcome.skill, outcome.errors.len());
        for e in &outcome.errors {
            eprintln!("  - {}", e);
        }
        std::process::exit(1);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_skill_md_errors() {
        let dir = assert_fs::TempDir::new().unwrap();
        let err = validate_skill("nope", dir.path()).unwrap_err();
        assert!(err.to_string().contains("skill not found"));
    }

    #[test]
    fn missing_required_fields_collected() {
        let dir = assert_fs::TempDir::new().unwrap();
        let skill_dir = dir.path().join(".agents/skills/x");
        std::fs::create_dir_all(&skill_dir).unwrap();
        // name present, description missing
        std::fs::write(skill_dir.join("SKILL.md"), "---\nname: x\n---\nbody\n").unwrap();
        let outcome = validate_skill("x", dir.path()).unwrap();
        assert!(!outcome.frontmatter_ok);
        assert!(!outcome.errors.is_empty());
    }

    #[test]
    fn valid_skill_md_passes() {
        let dir = assert_fs::TempDir::new().unwrap();
        let skill_dir = dir.path().join(".agents/skills/csv-parse");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: csv-parse\ndescription: Parse CSV.\nversion: 1.0.0\n---\nbody\n",
        )
        .unwrap();
        let outcome = validate_skill("csv-parse", dir.path()).unwrap();
        assert!(outcome.frontmatter_ok);
        assert!(outcome.errors.is_empty());
        assert_eq!(outcome.version.as_deref(), Some("1.0.0"));
    }
}
