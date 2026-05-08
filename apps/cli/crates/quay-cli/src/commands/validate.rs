//! Implementation of `quay validate <skill>`.

use quay_core::parse_skill;
use serde_json::json;
use std::path::Path;

/// Validate the frontmatter of a locally installed skill, offline.
pub fn run(skill: &str, project: &Path, json: bool) -> Result<(), Box<dyn std::error::Error>> {
    let path = project.join(".agents/skills").join(skill).join("SKILL.md");
    if !path.exists() {
        return Err(format!("skill not found: {}", path.display()).into());
    }
    let text = std::fs::read_to_string(&path)?;
    let (manifest, _body) = parse_skill(&text, &path.display().to_string())?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "skill": skill,
                "path": path.display().to_string(),
                "version": manifest.version,
                "valid": true,
            }))?
        );
    } else {
        println!("ok: {} v{}", skill, manifest.version);
    }
    Ok(())
}
