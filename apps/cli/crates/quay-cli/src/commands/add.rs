use quay_core::{
    add_plan::{
        build_plan, build_plan_with_prompt, collision_names, CollisionStrategy, SkillAction,
    },
    apply_all,
    push_log::PushLog,
    scanner::scan_local,
    CloneFetcher, Config, MirrorAction, QuayError, RegistryFetcher, SkillFileFetcher, SkillManager,
};
use std::path::Path;

/// Add one or more remote skills selected interactively via `dialoguer::MultiSelect`.
///
/// Fetches the registry for the configured (or specified) remote, then presents
/// a checkbox list so the user can pick which skills to install.
///
/// Returns `Err` immediately when stdin is not a TTY.
#[allow(clippy::too_many_arguments)]
pub fn run_interactive(
    remote: Option<&str>,
    force: bool,
    profile: Option<&str>,
    project: &Path,
    user_config: Option<&Path>,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let project_config = project.join(".quay/config.toml");
    let cfg = Config::load_resolved(user_config, Some(&project_config), profile)?;

    crate::commands::ensure_remotes_configured(&cfg)?;

    // Determine which remote to query.
    let remote_name = match remote {
        Some(r) => r.to_string(),
        None => cfg
            .default_remote()
            .map(|(n, _)| n.clone())
            .ok_or("no default remote configured — pass --remote=<name>")?,
    };
    let remote_cfg = cfg
        .remotes
        .get(&remote_name)
        .ok_or_else(|| format!("remote '{}' not configured", remote_name))?;
    let url = remote_cfg.url.clone();

    let mut fetcher = CloneFetcher::new();
    let registry = fetcher.fetch_registry(&url)?;
    let mut entries: Vec<(String, String, String)> = registry
        .skills
        .into_iter()
        .map(|(name, e)| (name, e.version, e.description))
        .collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));

    if entries.is_empty() {
        println!("(no skills on remote '{}')", remote_name);
        return Ok(());
    }

    let picks = crate::commands::interactive::pick_many(
        "Select skills to install (Space to toggle, Enter to confirm)",
        &entries,
        |(name, version, desc)| format!("{} v{} — {}", name, version, desc),
    )?;

    if picks.is_empty() {
        println!("(nothing selected)");
        return Ok(());
    }

    // Collect picked names in order.
    let pick_names: Vec<&str> = picks.iter().map(|&i| entries[i].0.as_str()).collect();

    // If force is already set, skip collision dialog.
    let plan: Vec<(String, SkillAction)> = if force {
        pick_names
            .iter()
            .map(|&n| (n.to_string(), SkillAction::UpdateForce))
            .collect()
    } else {
        // Compute collisions against local skills.
        let config_dir = crate::config_io::default_config_dir();
        let log = PushLog::load(config_dir.as_deref().unwrap_or(project), Some(project))
            .unwrap_or_default();
        let locals = scan_local(project, &log);
        let collisions = collision_names(&pick_names, &locals);

        if collisions.is_empty() {
            // No collisions — install everything.
            pick_names
                .iter()
                .map(|&n| (n.to_string(), SkillAction::Install))
                .collect()
        } else {
            // Show collision summary.
            println!(
                "{} of {} already exist locally:",
                collisions.len(),
                pick_names.len()
            );
            for col_name in &collisions {
                println!("  - {}", col_name);
            }
            println!();

            // Determine strategy — env var bypass for tests, dialoguer for real use.
            let strategy = resolve_collision_strategy()?;

            match strategy {
                CollisionStrategy::UpdateAll => {
                    build_plan(&pick_names, &locals, CollisionStrategy::UpdateAll)
                }
                CollisionStrategy::SkipAll => {
                    build_plan(&pick_names, &locals, CollisionStrategy::SkipAll)
                }
                CollisionStrategy::PromptEach => {
                    build_plan_with_prompt(&pick_names, &locals, |name, is_modified| {
                        let label = if is_modified {
                            format!("skill `{}` exists locally (modified). What to do?", name)
                        } else {
                            format!("skill `{}` exists locally. What to do?", name)
                        };
                        let choices = ["Update (overwrite from remote)", "Skip (keep local)"];
                        // In test environments dialoguer is bypassed via env var.
                        let idx = dialoguer::Select::new()
                            .with_prompt(label)
                            .items(&choices)
                            .default(0)
                            .interact()
                            .unwrap_or(1); // default to Skip on error
                        if idx == 0 {
                            SkillAction::UpdateForce
                        } else {
                            SkillAction::Skip
                        }
                    })
                }
            }
        }
    };

    // Execute plan.
    let f = CloneFetcher::new();
    let mut installed = 0usize;
    let mut updated = 0usize;
    let mut skipped = 0usize;
    let mut failed = 0usize;

    for (skill_name, action) in &plan {
        match action {
            SkillAction::Skip => {
                if !json {
                    println!("- {} skipped", skill_name);
                }
                skipped += 1;
            }
            SkillAction::Install | SkillAction::UpdateForce => {
                let do_force = matches!(action, SkillAction::UpdateForce);
                match run_with(
                    &cfg,
                    &f,
                    &f,
                    skill_name,
                    Some(remote_name.as_str()),
                    do_force,
                    project,
                    json,
                ) {
                    Ok(()) => {
                        if do_force {
                            if !json {
                                println!("\u{2713} {} updated", skill_name);
                            }
                            updated += 1;
                        } else {
                            if !json {
                                println!("\u{2713} {} installed (new)", skill_name);
                            }
                            installed += 1;
                        }
                    }
                    Err(e) => {
                        eprintln!("\u{2717} {}: {}", skill_name, e);
                        failed += 1;
                    }
                }
            }
        }
    }

    if !json && plan.len() > 1 {
        // Print summary only for multi-pick.
        let mut parts = Vec::new();
        if updated > 0 {
            parts.push(format!("{} updated", updated));
        }
        if installed > 0 {
            parts.push(format!("{} newly installed", installed));
        }
        if skipped > 0 {
            parts.push(format!("{} skipped", skipped));
        }
        if failed > 0 {
            parts.push(format!("{} failed", failed));
        }
        if !parts.is_empty() {
            println!("{}", parts.join(", "));
        }
    } else if !json && plan.len() == 1 && failed == 0 {
        // Single pick: legacy "installed N of N selected"
        let ok = installed + updated;
        println!("installed {} of {} selected", ok, ok + failed);
    }

    Ok(())
}

/// Resolve the collision strategy for the bulk-add dialog.
///
/// In test mode (env var `QUAY_TEST_COLLISION_STRATEGY` set), parses the
/// strategy directly.  In real interactive use, presents a `dialoguer::Select`.
fn resolve_collision_strategy() -> Result<CollisionStrategy, Box<dyn std::error::Error>> {
    // Test bypass: QUAY_TEST_COLLISION_STRATEGY=update_all|skip_all|prompt_each
    if let Ok(val) = std::env::var("QUAY_TEST_COLLISION_STRATEGY") {
        return match val.to_lowercase().as_str() {
            "update_all" => Ok(CollisionStrategy::UpdateAll),
            "skip_all" => Ok(CollisionStrategy::SkipAll),
            "prompt_each" => Ok(CollisionStrategy::PromptEach),
            other => Err(format!("unknown QUAY_TEST_COLLISION_STRATEGY={}", other).into()),
        };
    }

    let options = [
        "Update all (overwrite from remote)",
        "Skip all (only install new ones)",
        "Prompt per skill",
    ];
    let idx = dialoguer::Select::new()
        .with_prompt("What should we do with the existing ones?")
        .items(&options)
        .default(0)
        .interact()?;

    Ok(match idx {
        0 => CollisionStrategy::UpdateAll,
        1 => CollisionStrategy::SkipAll,
        _ => CollisionStrategy::PromptEach,
    })
}

pub fn run(
    skill: &str,
    remote: Option<&str>,
    force: bool,
    profile: Option<&str>,
    project: &Path,
    user_config: Option<&Path>,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let project_config = project.join(".quay/config.toml");
    let cfg = Config::load_resolved(user_config, Some(&project_config), profile)?;

    crate::commands::ensure_remotes_configured(&cfg)?;

    let f = CloneFetcher::new();
    run_with(&cfg, &f, &f, skill, remote, force, project, json)
}

#[allow(clippy::too_many_arguments)]
fn run_with<R: RegistryFetcher, F: SkillFileFetcher>(
    cfg: &Config,
    reg_fetcher: &R,
    file_fetcher: &F,
    skill: &str,
    remote: Option<&str>,
    force: bool,
    project: &Path,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let mgr = SkillManager::new(cfg, reg_fetcher, file_fetcher, project.to_path_buf());
    if force {
        mgr.add_with_force(skill, remote, true)?;
    } else {
        mgr.add(skill, remote)?;
    }
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "action": "installed",
                "skill": skill,
            }))?
        );
    } else {
        println!("installed {}", skill);
    }
    apply_mirrors_after_install(cfg, project, skill, json);
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
