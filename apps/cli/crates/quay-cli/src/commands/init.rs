use quay_core::Config;
use serde_json::json;
use std::path::Path;

pub fn run(project: &Path, json: bool) -> Result<(), Box<dyn std::error::Error>> {
    let config_path = project.join(".quay/config.toml");
    let skills_dir = project.join(".agents/skills");

    let created_config = !config_path.exists();
    if created_config {
        Config::default().write(&config_path)?;
    }
    std::fs::create_dir_all(&skills_dir)?;

    if json {
        let out = json!({
            "config_path": config_path.display().to_string(),
            "skills_dir": skills_dir.display().to_string(),
            "created_config": created_config,
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else if created_config {
        println!("created {}", config_path.display());
        println!("ensured {}", skills_dir.display());
    } else {
        println!("config already exists at {}", config_path.display());
        println!("ensured {}", skills_dir.display());
    }
    Ok(())
}
