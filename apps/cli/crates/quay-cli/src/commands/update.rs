//! `quay update` — pull latest version of installed skills from the remote registry.

use quay_core::{
    outdated_for_local, CloneFetcher, Config, OutdatedEntry, QuayError, RegistryFetcher,
    SkillFileFetcher, SkillManager,
};
use std::path::Path;

/// Update skills selected interactively via `dialoguer::MultiSelect`.
///
/// Shows only skills that have a newer version available (same list as
/// `quay outdated`).  Returns `Err` immediately when stdin is not a TTY.
#[allow(clippy::too_many_arguments)]
pub fn run_interactive(
    dry_run: bool,
    profile: Option<&str>,
    project: &Path,
    user_config: Option<&Path>,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let project_config = project.join(".quay/config.toml");
    let cfg = Config::load_resolved(user_config, Some(&project_config), profile)?;
    let config_dir = user_config.and_then(|p| p.parent());

    let f = CloneFetcher::new();
    let candidates: Vec<OutdatedEntry> = outdated_for_local(project, config_dir, &cfg, &f)?
        .into_iter()
        .collect();

    if candidates.is_empty() {
        println!("(everything up to date)");
        return Ok(());
    }

    let picks = crate::commands::interactive::pick_many(
        "Select skills to update (Space to toggle, Enter to confirm)",
        &candidates,
        |e| format!("{} → {} (from {})", e.name, e.available, e.remote),
    )?;

    if picks.is_empty() {
        println!("(nothing selected)");
        return Ok(());
    }

    if dry_run {
        for idx in &picks {
            let e = &candidates[*idx];
            println!(
                "would update {} → {} (from {})",
                e.name, e.available, e.remote
            );
        }
        return Ok(());
    }

    let mgr = SkillManager::new(&cfg, &f, &f, project.to_path_buf());
    let mut ok = 0usize;
    let mut fail = 0usize;
    for idx in &picks {
        let e = &candidates[*idx];
        match mgr.update_one(&e.name) {
            Ok(_) => {
                if !json {
                    println!("\u{2713} {} → {}", e.name, e.available);
                }
                ok += 1;
            }
            Err(err) => {
                eprintln!("\u{2717} {}: {}", e.name, err);
                fail += 1;
            }
        }
    }

    if !json {
        println!("updated {} of {} selected", ok, ok + fail);
    }
    Ok(())
}

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
    let config_dir = user_config.and_then(|p| p.parent());

    let f = CloneFetcher::new();
    run_with(&cfg, &f, &f, skill, dry_run, project, config_dir, json)?;

    // Keep the lockfile current if this project uses one.
    if project.join(quay_core::lock::LOCKFILE_NAME).exists() {
        let _ = crate::commands::lock::regenerate(project);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn run_with<R: RegistryFetcher, F: SkillFileFetcher>(
    cfg: &Config,
    reg_fetcher: &R,
    file_fetcher: &F,
    skill: Option<&str>,
    dry_run: bool,
    project: &Path,
    config_dir: Option<&Path>,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let candidates: Vec<OutdatedEntry> = outdated_for_local(project, config_dir, cfg, reg_fetcher)?
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
                    "would update {} to {} (from {})",
                    r.name, r.available, r.remote
                );
            }
        }
        return Ok(());
    }

    let mgr = SkillManager::new(cfg, reg_fetcher, file_fetcher, project.to_path_buf());
    let mut updated: Vec<&OutdatedEntry> = Vec::new();
    for cand in &candidates {
        match mgr.update_one(&cand.name) {
            Ok(_) => updated.push(cand),
            Err(QuayError::RemoteUnknown(remote)) => {
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
        println!("{}", serde_json::to_string_pretty(&updated)?);
    } else if updated.is_empty() {
        println!("(everything up to date)");
    } else {
        for r in &updated {
            println!("updated {} to {}", r.name, r.available);
        }
    }
    Ok(())
}
