//! Implementation of `quay push <skill>`.

use crate::args::BumpArg;
use quay_core::{BumpKind, Config, GhCliOpener, GitShellClient, QuayError, SkillPusher};
use serde_json::json;
use std::path::Path;

/// All information produced by a successful [`push_skill`] call.
#[derive(Debug)]
pub struct PushOutcome {
    /// The skill name as supplied.
    pub skill: String,
    /// Name of the remote the skill was pushed to.
    pub remote: String,
    /// Git branch that was created and pushed.
    pub branch: String,
    /// Semver version string that was written to the hub.
    pub version: String,
    /// URL of the PR that was opened (or a hint URL for manual creation).
    pub pr_url: String,
    /// `true` when the PR was created automatically by `gh pr create`.
    pub pr_auto_created: bool,
}

/// Push a local skill to a hub via PR without any output side-effects.
///
/// Returns a [`PushOutcome`] on success so the caller (CLI wrapper or TUI)
/// can decide how to present the result.
pub fn push_skill(
    skill: &str,
    remote: Option<&str>,
    bump: BumpKind,
    profile: Option<&str>,
    project: &Path,
    user_config: Option<&Path>,
) -> Result<PushOutcome, QuayError> {
    let project_config = project.join(".quay/config.toml");
    let cfg = Config::load_resolved(user_config, Some(&project_config), profile)?;
    ensure_remotes_configured_core(&cfg)?;

    let git = GitShellClient;
    let opener = GhCliOpener;
    let clone_root = std::env::temp_dir().join(format!("quay-push-{}", std::process::id()));
    std::fs::create_dir_all(&clone_root).map_err(|source| QuayError::Io {
        path: clone_root.display().to_string(),
        source,
    })?;

    let pusher = SkillPusher {
        config: &cfg,
        git: &git,
        opener: &opener,
        project_root: project.to_path_buf(),
        author: None,
    };
    let result = pusher.push(skill, remote, bump, &clone_root)?;

    // Best-effort cleanup of the temp clone tree.
    let _ = std::fs::remove_dir_all(&clone_root);

    Ok(PushOutcome {
        skill: skill.to_string(),
        remote: result.remote,
        branch: result.branch,
        version: result.version,
        pr_url: result.pr.url,
        pr_auto_created: result.pr.auto_created,
    })
}

/// Returns `Err` when the merged config has zero remotes configured.
fn ensure_remotes_configured_core(cfg: &Config) -> Result<(), QuayError> {
    if cfg.remotes.is_empty() {
        return Err(QuayError::ConfigValidation(
            "no remotes configured — run `quay remote add <name> <url> --default` first".into(),
        ));
    }
    Ok(())
}

/// Push a local skill to a hub via PR.
pub fn run(
    skill: &str,
    remote: Option<&str>,
    bump: BumpArg,
    profile: Option<&str>,
    project: &Path,
    user_config: Option<&Path>,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let bump_kind = match bump {
        BumpArg::Patch => BumpKind::Patch,
        BumpArg::Minor => BumpKind::Minor,
        BumpArg::Major => BumpKind::Major,
        BumpArg::AsWritten => BumpKind::AsWritten,
    };

    let outcome = push_skill(skill, remote, bump_kind, profile, project, user_config)?;

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "skill": outcome.skill,
                "remote": outcome.remote,
                "branch": outcome.branch,
                "version": outcome.version,
                "pr_url": outcome.pr_url,
                "pr_auto_created": outcome.pr_auto_created,
            }))?
        );
    } else if outcome.pr_auto_created {
        println!(
            "pushed {} → {}\nPR opened: {}",
            outcome.branch, outcome.remote, outcome.pr_url
        );
    } else {
        println!(
            "pushed {} → {}\nopen the PR manually: {}",
            outcome.branch, outcome.remote, outcome.pr_url
        );
    }

    Ok(())
}
