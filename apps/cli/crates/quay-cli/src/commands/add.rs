#[cfg(debug_assertions)]
use quay_core::GithubRawFetcherWithBase;
use quay_core::{
    apply_all, Config, GithubRawFetcher, MirrorAction, QuayError, RegistryFetcher,
    SkillFileFetcher, SkillManager,
};
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

    let branch = std::env::var("QUAY_GITHUB_BRANCH").unwrap_or_else(|_| "main".into());

    #[cfg(debug_assertions)]
    if let Ok(base) = std::env::var("QUAY_GITHUB_BASE_URL") {
        let f = GithubRawFetcherWithBase::new(branch, base);
        return run_with(&cfg, &f, &f, skill, remote, project, json);
    }

    let f = GithubRawFetcher::new(branch);
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
    let locked = mgr.add(skill, remote)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&locked)?);
    } else {
        println!(
            "installed {}@{} from {}",
            skill, locked.version, locked.remote
        );
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
