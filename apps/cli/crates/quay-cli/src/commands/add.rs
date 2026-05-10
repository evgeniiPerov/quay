use quay_core::{
    apply_all, CloneFetcher, Config, MirrorAction, QuayError, RegistryFetcher, SkillFileFetcher,
    SkillManager,
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

    let f = CloneFetcher::new();
    let mut ok = 0usize;
    let mut fail = 0usize;
    for idx in &picks {
        let (skill_name, _, _) = &entries[*idx];
        match run_with(
            &cfg,
            &f,
            &f,
            skill_name,
            Some(remote_name.as_str()),
            force,
            project,
            json,
        ) {
            Ok(()) => ok += 1,
            Err(e) => {
                eprintln!("\u{2717} {}: {}", skill_name, e);
                fail += 1;
            }
        }
    }

    if !json {
        println!("installed {} of {} selected", ok, ok + fail);
    }
    Ok(())
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
