//! Implementation of `quay push <skill>`.

use crate::args::BumpArg;
use quay_core::{BumpKind, Config, GhCliOpener, GitShellClient, SkillPusher};
use serde_json::json;
use std::path::Path;

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
    let project_config = project.join(".quay/config.toml");
    let cfg = Config::load_resolved(user_config, Some(&project_config), profile)?;
    crate::commands::ensure_remotes_configured(&cfg)?;

    let bump_kind = match bump {
        BumpArg::Patch => BumpKind::Patch,
        BumpArg::Minor => BumpKind::Minor,
        BumpArg::Major => BumpKind::Major,
        BumpArg::AsWritten => BumpKind::AsWritten,
    };

    let git = GitShellClient;
    let opener = GhCliOpener;
    let clone_root = std::env::temp_dir().join(format!("quay-push-{}", std::process::id()));
    std::fs::create_dir_all(&clone_root)?;

    let pusher = SkillPusher {
        config: &cfg,
        git: &git,
        opener: &opener,
        project_root: project.to_path_buf(),
        author: None,
    };
    let result = pusher.push(skill, remote, bump_kind, &clone_root)?;

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "skill": skill,
                "remote": result.remote,
                "branch": result.branch,
                "version": result.version,
                "pr_url": result.pr.url,
                "pr_auto_created": result.pr.auto_created,
            }))?
        );
    } else if result.pr.auto_created {
        println!(
            "pushed {} → {}\nPR opened: {}",
            result.branch, result.remote, result.pr.url
        );
    } else {
        println!(
            "pushed {} → {}\nopen the PR manually: {}",
            result.branch, result.remote, result.pr.url
        );
    }

    // Best-effort cleanup of the temp clone tree.
    let _ = std::fs::remove_dir_all(&clone_root);
    Ok(())
}
