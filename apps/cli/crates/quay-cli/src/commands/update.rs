//! `quay update` — pull latest version of installed skills from the remote registry.

use crate::commands::add::apply_mirrors_after_install;
use crate::commands::extras::{is_interrupt, Decider, ExtraPolicy};
use crate::commands::interactive::is_tty;
use quay_core::{
    outdated_for_local, CloneFetcher, Config, OutdatedEntry, QuayError, RegistryFetcher,
    SkillFileFetcher, SkillManager,
};
use serde::Serialize;
use std::path::Path;

/// One updated skill in `quay update --json`: the `outdated` row, plus what
/// the update deleted — scripts that pass `--delete-extra` need a record.
#[derive(Serialize)]
struct Updated<'a> {
    #[serde(flatten)]
    entry: &'a OutdatedEntry,
    deleted_extras: Vec<String>,
}

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
    policy: ExtraPolicy,
) -> Result<(), Box<dyn std::error::Error>> {
    let project_config = project.join(".quay/config.toml");
    let cfg = Config::load_resolved(user_config, Some(&project_config), profile)?;
    let config_dir = user_config.and_then(|p| p.parent());

    let f = CloneFetcher::new();
    let candidates: Vec<OutdatedEntry> = outdated_for_local(project, config_dir, &cfg, &f)?
        .into_iter()
        .filter(|e| e.upgrade_available)
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
    let decider = Decider::new(policy, is_tty() && !json);
    let mut ok = 0usize;
    let mut interrupted = None;
    for idx in &picks {
        let e = &candidates[*idx];
        match mgr.update_one_with_extras(&e.name, &|s, x| decider.decide(s, x)) {
            Ok(_) => {
                if !json {
                    println!("\u{2713} {} → {}", e.name, e.available);
                }
                apply_mirrors_after_install(&cfg, project, &e.name, json);
                ok += 1;
            }
            // This loop counts failures and keeps going; a cancellation must
            // stop it instead of re-prompting the next skill.
            Err(err) if is_interrupt(&err) => {
                interrupted = Some(err);
                break;
            }
            Err(err) => {
                eprintln!("\u{2717} {}: {}", e.name, err);
            }
        }
    }

    if !json {
        println!("updated {} of {} selected", ok, picks.len());
    }
    // Keep the lockfile current if this project uses one (best-effort).
    crate::commands::lock::regenerate_if_present(project);
    // A script wrapping an aborted run must not see success with skills left
    // untouched.
    match interrupted {
        Some(err) => Err(err.into()),
        None => Ok(()),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn run(
    skill: Option<&str>,
    dry_run: bool,
    profile: Option<&str>,
    project: &Path,
    user_config: Option<&Path>,
    json: bool,
    policy: ExtraPolicy,
) -> Result<(), Box<dyn std::error::Error>> {
    let project_config = project.join(".quay/config.toml");
    let cfg = Config::load_resolved(user_config, Some(&project_config), profile)?;
    let config_dir = user_config.and_then(|p| p.parent());

    let f = CloneFetcher::new();
    let decider = Decider::new(policy, is_tty() && !json);
    run_with(
        &cfg, &f, &f, skill, dry_run, project, config_dir, json, &decider,
    )?;

    // Keep the lockfile current if this project uses one (best-effort).
    crate::commands::lock::regenerate_if_present(project);
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
    decider: &Decider,
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
    let mut updated: Vec<Updated> = Vec::new();
    for cand in &candidates {
        match mgr.update_one_with_extras(&cand.name, &|s, e| decider.decide(s, e)) {
            Ok(_) => {
                // Copy mirrors hold their own bytes; without this they keep the
                // old version and any extra the update just deleted.
                apply_mirrors_after_install(cfg, project, &cand.name, json);
                updated.push(Updated {
                    entry: cand,
                    deleted_extras: decider.deleted_for(&cand.name),
                });
            }
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
            println!("updated {} to {}", r.entry.name, r.entry.available);
        }
    }
    Ok(())
}
