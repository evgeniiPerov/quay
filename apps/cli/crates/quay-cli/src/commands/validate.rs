//! Implementation of `quay validate <skill>`.

use quay_core::{parse_skill, QuayError};
use serde_json::json;
use std::path::Path;

/// Controls whether validation warnings are treated as hard errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidateMode {
    /// Default behaviour: collect warnings, do not fail the run.
    Soft,
    /// Treat any warning as a hard error (exit 1).
    Strict,
}

/// All information produced by a successful [`validate_skill`] call.
#[derive(Debug)]
pub struct ValidateOutcome {
    /// The skill name as supplied.
    pub skill: String,
    /// Whether the frontmatter parsed cleanly and all required fields are present.
    pub frontmatter_ok: bool,
    /// Semver version string from the frontmatter; `None` when validation failed.
    pub version: Option<String>,
    /// Diagnostics collected during validation.  In `Soft` mode these are warnings;
    /// in `Strict` mode these become reasons to fail.
    pub warnings: Vec<String>,
    /// In `Strict` mode: `true` iff any warning was collected. In `Soft` mode:
    /// always `false`.
    pub strict_failed: bool,
}

/// Validate a locally installed skill's `SKILL.md` without any output side-effects.
///
/// Returns `Ok(ValidateOutcome)` in all cases where the file can be read;
/// `Err` only when the file cannot be found or opened (I/O failure).
pub fn validate_skill(
    skill: &str,
    project: &Path,
    mode: ValidateMode,
) -> Result<ValidateOutcome, QuayError> {
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

    let mut warnings = Vec::new();
    let (frontmatter_ok, version) = match parse_skill(&text, &path.display().to_string()) {
        Ok((manifest, _body)) => (true, Some(manifest.version)),
        Err(e) => {
            warnings.push(e.to_string());
            (false, None)
        }
    };

    let strict_failed = matches!(mode, ValidateMode::Strict) && !warnings.is_empty();

    Ok(ValidateOutcome {
        skill: skill.to_string(),
        frontmatter_ok,
        version,
        warnings,
        strict_failed,
    })
}

/// Validate the frontmatter of a locally installed skill, offline.
pub fn run(
    skill: &str,
    project: &Path,
    json: bool,
    strict: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let mode = if strict {
        ValidateMode::Strict
    } else {
        ValidateMode::Soft
    };
    let outcome = validate_skill(skill, project, mode)?;

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "skill": outcome.skill,
                "ok": outcome.warnings.is_empty(),
                "strict_failed": outcome.strict_failed,
                "warnings": outcome.warnings,
            }))?
        );
    } else if outcome.warnings.is_empty() {
        println!(
            "ok: {} v{}",
            outcome.skill,
            outcome.version.as_deref().unwrap_or("unknown")
        );
    } else {
        // Soft mode prints to stderr but exits 0; strict bumps to fail-and-exit-1.
        if matches!(mode, ValidateMode::Soft) {
            eprintln!(
                "warning: {} has {} note(s):",
                outcome.skill,
                outcome.warnings.len()
            );
        } else {
            eprintln!("{} has {} error(s):", outcome.skill, outcome.warnings.len());
        }
        for w in &outcome.warnings {
            eprintln!("  - {w}");
        }
    }

    if outcome.strict_failed {
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
        let err = validate_skill("nope", dir.path(), ValidateMode::Soft).unwrap_err();
        assert!(err.to_string().contains("skill not found"));
    }

    #[test]
    fn soft_mode_returns_warnings_but_does_not_fail_on_missing_frontmatter() {
        let dir = assert_fs::TempDir::new().unwrap();
        let skill_dir = dir.path().join(".agents/skills/freestyle");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(skill_dir.join("SKILL.md"), "Just markdown.\n").unwrap();
        let outcome = validate_skill("freestyle", dir.path(), ValidateMode::Soft).unwrap();
        assert!(!outcome.frontmatter_ok);
        assert!(
            !outcome.warnings.is_empty(),
            "expected at least one warning"
        );
        assert!(!outcome.strict_failed, "soft mode never fails");
    }

    #[test]
    fn strict_mode_flags_missing_frontmatter() {
        let dir = assert_fs::TempDir::new().unwrap();
        let skill_dir = dir.path().join(".agents/skills/freestyle");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(skill_dir.join("SKILL.md"), "Just markdown.\n").unwrap();
        let outcome = validate_skill("freestyle", dir.path(), ValidateMode::Strict).unwrap();
        assert!(outcome.strict_failed);
        assert!(!outcome.warnings.is_empty());
    }

    #[test]
    fn missing_required_fields_warned_in_soft_mode() {
        let dir = assert_fs::TempDir::new().unwrap();
        let skill_dir = dir.path().join(".agents/skills/x");
        std::fs::create_dir_all(&skill_dir).unwrap();
        // name present, description missing
        std::fs::write(skill_dir.join("SKILL.md"), "---\nname: x\n---\nbody\n").unwrap();
        let outcome = validate_skill("x", dir.path(), ValidateMode::Soft).unwrap();
        assert!(!outcome.frontmatter_ok);
        assert!(!outcome.warnings.is_empty());
        assert!(!outcome.strict_failed);
    }

    #[test]
    fn valid_skill_md_passes_in_both_modes() {
        let dir = assert_fs::TempDir::new().unwrap();
        let skill_dir = dir.path().join(".agents/skills/csv-parse");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: csv-parse\ndescription: Parse CSV.\nversion: 1.0.0\n---\nbody\n",
        )
        .unwrap();

        for mode in [ValidateMode::Soft, ValidateMode::Strict] {
            let outcome = validate_skill("csv-parse", dir.path(), mode).unwrap();
            assert!(outcome.frontmatter_ok);
            assert!(outcome.warnings.is_empty());
            assert!(!outcome.strict_failed);
            assert_eq!(outcome.version.as_deref(), Some("1.0.0"));
        }
    }
}
