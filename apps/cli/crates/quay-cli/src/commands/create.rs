//! Implementation of `quay create <name>`.

use quay_core::{Config, QuayError};
use serde_json::json;
use std::path::{Path, PathBuf};

const TEMPLATE: &str = "---\nname: {{NAME}}\ndescription: \nversion: 0.1.0\ncategory: \ntags: []\nauthor: {{AUTHOR}}\n---\n\n# {{NAME}}\n\nDescribe your skill here.\n";

/// All information produced by a successful [`scaffold`] call.
#[derive(Debug)]
pub struct CreateOutcome {
    /// The skill name as supplied (already validated to kebab-case).
    pub skill: String,
    /// Absolute path to the newly written `SKILL.md`.
    pub skill_md_path: PathBuf,
    /// Author identity that was written into the frontmatter.
    pub author: String,
}

/// Scaffold a new local `SKILL.md` template without any I/O side-effects
/// beyond writing the file.  Returns a [`CreateOutcome`] on success so the
/// caller (CLI wrapper or TUI) can decide how to present the result.
pub fn scaffold(
    name: &str,
    author: Option<&str>,
    profile: Option<&str>,
    project: &Path,
    user_config: Option<&Path>,
) -> Result<CreateOutcome, QuayError> {
    if !is_kebab_case(name) {
        return Err(QuayError::InvalidInput(format!(
            "skill name '{}' must be kebab-case (lowercase letters/digits, hyphens)",
            name
        )));
    }
    let project_config = project.join(".quay/config.toml");
    let cfg = Config::load_resolved(user_config, Some(&project_config), profile)?;

    let resolved_author = match author {
        Some(a) => a.to_string(),
        None => cfg
            .user
            .email
            .clone()
            .unwrap_or_else(|| "you@example.com".into()),
    };

    let skill_dir = project.join(".agents/skills").join(name);
    if skill_dir.exists() {
        return Err(QuayError::AlreadyExists(skill_dir.display().to_string()));
    }
    std::fs::create_dir_all(&skill_dir).map_err(|source| QuayError::Io {
        path: skill_dir.display().to_string(),
        source,
    })?;
    let body = TEMPLATE
        .replace("{{NAME}}", name)
        .replace("{{AUTHOR}}", &resolved_author);
    let md = skill_dir.join("SKILL.md");
    std::fs::write(&md, &body).map_err(|source| QuayError::Io {
        path: md.display().to_string(),
        source,
    })?;

    Ok(CreateOutcome {
        skill: name.to_string(),
        skill_md_path: md,
        author: resolved_author,
    })
}

/// Scaffold a new local SKILL.md template at `<project>/.agents/skills/<name>/SKILL.md`.
pub fn run(
    name: &str,
    author: Option<&str>,
    profile: Option<&str>,
    project: &Path,
    user_config: Option<&Path>,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let outcome = scaffold(name, author, profile, project, user_config)?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "skill": outcome.skill,
                "path": outcome.skill_md_path.display().to_string(),
            }))?
        );
    } else {
        println!("created {}", outcome.skill_md_path.display());
        println!(
            "edit it, then `quay validate {}` and `quay push {}` to contribute.",
            outcome.skill, outcome.skill
        );
    }
    Ok(())
}

fn is_kebab_case(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !s.starts_with('-')
        && !s.ends_with('-')
        && !s.contains("--")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_kebab_case() {
        let dir = assert_fs::TempDir::new().unwrap();
        let err = scaffold("BadName", None, None, dir.path(), None).unwrap_err();
        assert!(err.to_string().contains("kebab-case"));
    }

    #[test]
    fn writes_skill_md_with_author() {
        let dir = assert_fs::TempDir::new().unwrap();
        let outcome = scaffold("my-skill", Some("a@b.com"), None, dir.path(), None).unwrap();
        let body = std::fs::read_to_string(&outcome.skill_md_path).unwrap();
        assert!(body.contains("name: my-skill"));
        assert!(body.contains("author: a@b.com"));
    }

    #[test]
    fn errors_when_skill_dir_already_exists() {
        let dir = assert_fs::TempDir::new().unwrap();
        // Create it once.
        scaffold("existing-skill", Some("a@b.com"), None, dir.path(), None).unwrap();
        // Second call must fail with AlreadyExists.
        let err = scaffold("existing-skill", Some("a@b.com"), None, dir.path(), None).unwrap_err();
        assert!(matches!(err, QuayError::AlreadyExists(_)));
    }
}
