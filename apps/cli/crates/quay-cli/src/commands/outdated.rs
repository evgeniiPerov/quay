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
    // Content drift earns a row of its own: bumping `version` on push is a
    // convention quay does not enforce, so a hub edit at an unchanged version
    // is invisible to the semver comparison but still leaves the local copy
    // stale.
    let stale: Vec<&OutdatedEntry> = rows
        .iter()
        .filter(|r| r.upgrade_available || r.content_drift)
        .collect();

    if json {
        println!("{}", serde_json::to_string_pretty(&stale)?);
    } else if stale.is_empty() {
        println!("(everything up to date)");
    } else {
        let mut drift_only = false;
        for r in &stale {
            if r.upgrade_available {
                println!(
                    "{:<24} local -> {}    (from {})",
                    r.name, r.available, r.remote
                );
            } else {
                drift_only = true;
                println!(
                    "{:<24} differs from hub at {}    (from {})",
                    r.name, r.available, r.remote
                );
            }
        }
        if drift_only {
            // `quay update` acts on semver upgrades only, so pointing at it here
            // would send the user somewhere that does nothing.
            println!(
                "\nSkills marked 'differs from hub' have no version change; \
                 either side may have been edited.\n\
                 Take the hub copy with: quay add <name> --force"
            );
        }
    }
    Ok(())
}
