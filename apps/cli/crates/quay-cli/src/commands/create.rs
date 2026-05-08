//! Implementation of `quay create <name>`.

use quay_core::Config;
use serde_json::json;
use std::path::Path;

const TEMPLATE: &str = "---\nname: {{NAME}}\ndescription: \nversion: 0.1.0\ncategory: \ntags: []\nauthor: {{AUTHOR}}\n---\n\n# {{NAME}}\n\nDescribe your skill here.\n";

/// Scaffold a new local SKILL.md template at `<project>/.agents/skills/<name>/SKILL.md`.
pub fn run(
    name: &str,
    author: Option<&str>,
    profile: Option<&str>,
    project: &Path,
    user_config: Option<&Path>,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    if !is_kebab_case(name) {
        return Err(format!(
            "skill name '{}' must be kebab-case (lowercase letters/digits, hyphens)",
            name
        )
        .into());
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
        return Err(format!("{} already exists", skill_dir.display()).into());
    }
    std::fs::create_dir_all(&skill_dir)?;
    let body = TEMPLATE
        .replace("{{NAME}}", name)
        .replace("{{AUTHOR}}", &resolved_author);
    let md = skill_dir.join("SKILL.md");
    std::fs::write(&md, body)?;

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "skill": name,
                "path": md.display().to_string(),
            }))?
        );
    } else {
        println!("created {}", md.display());
        println!(
            "edit it, then `quay validate {}` and `quay push {}` to contribute.",
            name, name
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
