//! `quay outdated` — compare local skill versions against remote registry.
//!
//! Plan 10: no lockfile. Compares local SKILL.md content against each
//! remote's registry.json on the fly.

use quay_core::{outdated_for_local, CloneFetcher, Config, OutdatedEntry, RegistryFetcher};
use std::path::Path;

pub fn run(
    project: &Path,
    profile: Option<&str>,
    user_config: Option<&Path>,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let project_config = project.join(".quay/config.toml");
    let cfg = Config::load_resolved(user_config, Some(&project_config), profile)?;
    let config_dir = user_config.and_then(|p| p.parent());

    let f = CloneFetcher::new();
    run_with(&cfg, &f, project, config_dir, json)
}

fn run_with<R: RegistryFetcher>(
    cfg: &Config,
    fetcher: &R,
    project: &Path,
    config_dir: Option<&Path>,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let rows = outdated_for_local(project, config_dir, cfg, fetcher)?;
    let stale: Vec<&OutdatedEntry> = rows.iter().filter(|r| r.upgrade_available).collect();

    if json {
        println!("{}", serde_json::to_string_pretty(&stale)?);
    } else if stale.is_empty() {
        println!("(everything up to date)");
    } else {
        for r in &stale {
            println!(
                "{:<24} local -> {}    (from {})",
                r.name, r.available, r.remote
            );
        }
    }
    Ok(())
}
