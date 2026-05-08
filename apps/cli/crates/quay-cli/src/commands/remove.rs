use quay_core::{Config, GithubRawFetcher, SkillManager};
use serde_json::json;
use std::path::Path;

pub fn run(
    skill: &str,
    profile: Option<&str>,
    project: &Path,
    user_config: Option<&Path>,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let project_config = project.join(".quay/config.toml");
    let cfg = Config::load_resolved(user_config, Some(&project_config), profile)?;
    // remove only touches the lockfile + filesystem; we still need the trait bounds
    // satisfied so we pass a dummy fetcher that's never called.
    let f = GithubRawFetcher::new("main");
    let mgr = SkillManager::new(&cfg, &f, &f, project.to_path_buf());
    mgr.remove(skill)?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({"action": "removed", "skill": skill}))?
        );
    } else {
        println!("removed {}", skill);
    }
    Ok(())
}
