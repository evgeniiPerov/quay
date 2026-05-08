#[cfg(debug_assertions)]
use quay_core::GithubRawFetcherWithBase;
use quay_core::{outdated, Config, GithubRawFetcher, Lockfile, OutdatedEntry, RegistryFetcher};
use std::path::Path;

pub fn run(
    project: &Path,
    profile: Option<&str>,
    user_config: Option<&Path>,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let project_config = project.join(".quay/config.toml");
    let cfg = Config::load_resolved(user_config, Some(&project_config), profile)?;
    let lock = Lockfile::load_or_default(&project.join(".agents/skills.lock.json"))?;

    if lock.skills.is_empty() {
        if json {
            println!("[]");
        } else {
            println!("(no skills installed)");
        }
        return Ok(());
    }

    let branch = std::env::var("QUAY_GITHUB_BRANCH").unwrap_or_else(|_| "main".into());

    #[cfg(debug_assertions)]
    if let Ok(base) = std::env::var("QUAY_GITHUB_BASE_URL") {
        let f = GithubRawFetcherWithBase::new(branch, base);
        return run_with(&cfg, &f, &lock, json);
    }

    let f = GithubRawFetcher::new(branch);
    run_with(&cfg, &f, &lock, json)
}

fn run_with<R: RegistryFetcher>(
    cfg: &Config,
    fetcher: &R,
    lock: &Lockfile,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let rows = outdated(cfg, fetcher, lock)?;
    let stale: Vec<&OutdatedEntry> = rows.iter().filter(|r| r.upgrade_available).collect();

    if json {
        println!("{}", serde_json::to_string_pretty(&stale)?);
    } else if stale.is_empty() {
        println!("(everything up to date)");
    } else {
        for r in &stale {
            println!(
                "{:<24} {} -> {}    (from {})",
                r.name, r.installed, r.available, r.remote
            );
        }
    }
    Ok(())
}
