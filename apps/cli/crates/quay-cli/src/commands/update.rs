#[cfg(debug_assertions)]
use quay_core::GithubRawFetcherWithBase;
use quay_core::{
    apply_all, outdated, Config, GithubRawFetcher, Lockfile, MirrorAction, OutdatedEntry,
    QuayError, RegistryFetcher, SkillFileFetcher, SkillManager,
};
use std::path::Path;

pub fn run(
    skill: Option<&str>,
    dry_run: bool,
    profile: Option<&str>,
    project: &Path,
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
        return run_with(&cfg, &f, &f, &lock, skill, dry_run, project, json);
    }

    let f = GithubRawFetcher::new(branch);
    run_with(&cfg, &f, &f, &lock, skill, dry_run, project, json)
}

#[allow(clippy::too_many_arguments)]
fn run_with<R: RegistryFetcher, F: SkillFileFetcher>(
    cfg: &Config,
    reg_fetcher: &R,
    file_fetcher: &F,
    lock: &Lockfile,
    skill: Option<&str>,
    dry_run: bool,
    project: &Path,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    // Always compute the set of entries that need updating up front.
    let candidates: Vec<OutdatedEntry> = outdated(cfg, reg_fetcher, lock)?
        .into_iter()
        .filter(|r| match skill {
            Some(s) => r.name == s && r.upgrade_available,
            None => r.upgrade_available,
        })
        .collect();

    if dry_run {
        if json {
            println!("{}", serde_json::to_string_pretty(&candidates)?);
        } else if candidates.is_empty() {
            println!("(nothing would change)");
        } else {
            for r in &candidates {
                println!(
                    "would update {} from {} to {} (from {})",
                    r.name, r.installed, r.available, r.remote
                );
            }
        }
        return Ok(());
    }

    let mgr = SkillManager::new(cfg, reg_fetcher, file_fetcher, project.to_path_buf());
    let mut updated_rows: Vec<OutdatedEntry> = Vec::new();
    for cand in &candidates {
        match mgr.update_one(&cand.name) {
            Ok(Some(_)) => updated_rows.push(cand.clone()),
            Ok(None) => {} // race: registry stopped serving newer version mid-pass
            Err(quay_core::QuayError::RemoteUnknown(remote)) => {
                if !json {
                    eprintln!(
                        "warning: skipping {} — remote '{}' is no longer configured",
                        cand.name, remote
                    );
                }
            }
            Err(e) => return Err(e.into()),
        }
    }

    if json {
        println!("{}", serde_json::to_string_pretty(&updated_rows)?);
    } else if updated_rows.is_empty() {
        println!("(everything up to date)");
    } else {
        for r in &updated_rows {
            println!("updated {} from {} to {}", r.name, r.installed, r.available);
        }
    }

    for r in &updated_rows {
        apply_mirrors_after_install(cfg, project, &r.name, json);
    }
    Ok(())
}

fn apply_mirrors_after_install(cfg: &Config, project: &Path, skill: &str, json: bool) {
    if cfg.install.mirrors.is_empty() {
        return;
    }
    match apply_all(&cfg.install, project, skill, false) {
        Ok(actions) => {
            if json {
                return;
            }
            for action in &actions {
                match action {
                    MirrorAction::Created { path, strategy } => {
                        println!("  mirror: created {} ({:?})", path.display(), strategy);
                    }
                    MirrorAction::Replaced { path, strategy } => {
                        println!("  mirror: replaced {} ({:?})", path.display(), strategy);
                    }
                    MirrorAction::NoOp => {}
                }
            }
        }
        Err(QuayError::MirrorConflict { path, reason }) => {
            if !json {
                eprintln!(
                    "warning: mirror not applied at {}: {}. Run `quay link --force` to resolve.",
                    path, reason
                );
            }
        }
        Err(e) => {
            if !json {
                eprintln!("warning: mirror apply failed: {}", e);
            }
        }
    }
}
