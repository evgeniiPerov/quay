#[cfg(debug_assertions)]
use quay_core::GithubRawFetcherWithBase;
use quay_core::{Config, GithubRawFetcher, RegistryFetcher, SkillFileFetcher, SkillManager};
use std::path::Path;

pub fn run(
    project: &Path,
    profile: Option<&str>,
    user_config: Option<&Path>,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let project_config = project.join(".quay/config.toml");
    let cfg = Config::load_resolved(user_config, Some(&project_config), profile)?;

    let branch = std::env::var("QUAY_GITHUB_BRANCH").unwrap_or_else(|_| "main".into());

    #[cfg(debug_assertions)]
    if let Ok(base) = std::env::var("QUAY_GITHUB_BASE_URL") {
        let f = GithubRawFetcherWithBase::new(branch, base);
        return run_with(&cfg, &f, &f, project, json);
    }

    let f = GithubRawFetcher::new(branch);
    run_with(&cfg, &f, &f, project, json)
}

fn run_with<R: RegistryFetcher, F: SkillFileFetcher>(
    cfg: &Config,
    reg_fetcher: &R,
    file_fetcher: &F,
    project: &Path,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let mgr = SkillManager::new(cfg, reg_fetcher, file_fetcher, project.to_path_buf());
    let refetched = mgr.sync()?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({"refetched": refetched}))?
        );
    } else if refetched.is_empty() {
        println!("synced (no changes)");
    } else {
        for r in &refetched {
            println!("refetched {}/{}", r.skill, r.file);
        }
    }
    Ok(())
}
