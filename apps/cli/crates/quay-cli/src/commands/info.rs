use quay_core::{CloneFetcher, Config, RegistryFetcher, SkillFileFetcher, SkillManager};
use std::path::Path;

pub fn run(
    skill: &str,
    remote: Option<&str>,
    profile: Option<&str>,
    project: &Path,
    user_config: Option<&Path>,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let project_config = project.join(".quay/config.toml");
    let cfg = Config::load_resolved(user_config, Some(&project_config), profile)?;
    crate::commands::ensure_remotes_configured(&cfg)?;

    let f = CloneFetcher::new();
    run_with(&cfg, &f, &f, skill, remote, project, json)
}

fn run_with<R: RegistryFetcher, F: SkillFileFetcher>(
    cfg: &Config,
    reg_fetcher: &R,
    file_fetcher: &F,
    skill: &str,
    remote: Option<&str>,
    project: &Path,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let mgr = SkillManager::new(cfg, reg_fetcher, file_fetcher, project.to_path_buf());
    let entry = mgr.info(skill, remote)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&entry)?);
    } else {
        println!("name:        {}", skill);
        println!("version:     {}", entry.version);
        println!("description: {}", entry.description);
        if let Some(cat) = &entry.category {
            println!("category:    {}", cat);
        }
        if !entry.tags.is_empty() {
            println!("tags:        {}", entry.tags.join(", "));
        }
        println!("path:        {}", entry.path);
        println!("sha:         {}", entry.sha);
        println!("files:");
        for f in &entry.files {
            println!("  - {}", f);
        }
    }
    Ok(())
}
