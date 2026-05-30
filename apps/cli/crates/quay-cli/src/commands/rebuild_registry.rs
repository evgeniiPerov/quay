//! `quay rebuild-registry [<remote>]` — clone the hub, regenerate `registry.json`
//! from disk truth (every `skills/<name>/SKILL.md` becomes one entry), commit
//! and push it back. Honours the remote's `push_mode`.

use quay_core::error::{QuayError, Result};
use quay_core::registry_builder::build_from_hub_clone;
use quay_core::{provider_for_remote, Config, GitClient, GitShellClient, PushMode};
use std::path::Path;

pub fn run(
    remote: Option<&str>,
    push_mode_override: Option<PushMode>,
    project: &Path,
    user_config: Option<&Path>,
    profile: Option<&str>,
    json: bool,
) -> Result<()> {
    let project_config = project.join(".quay/config.toml");
    let project_config_path = if project_config.exists() {
        Some(project_config.as_path())
    } else {
        None
    };
    let cfg = Config::load_resolved(user_config, project_config_path, profile)?;

    let (remote_name, remote_cfg) = match remote {
        Some(name) => {
            let r = cfg
                .remotes
                .get(name)
                .ok_or_else(|| QuayError::RemoteUnknown(name.into()))?;
            (name.to_string(), r.clone())
        }
        None => {
            let (name, r) = cfg.default_remote().ok_or_else(|| {
                QuayError::ConfigValidation("no default remote — pass <remote>".into())
            })?;
            (name.clone(), r.clone())
        }
    };

    let effective_mode = push_mode_override.unwrap_or(remote_cfg.push_mode);

    eprintln!(
        "rebuild-registry: cloning {} ({}) …",
        remote_name, remote_cfg.url
    );

    let git = GitShellClient;
    let clone_root = std::env::temp_dir().join(format!("quay-rebuild-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&clone_root);
    std::fs::create_dir_all(&clone_root).map_err(|source| QuayError::Io {
        path: clone_root.display().to_string(),
        source,
    })?;
    let hub_clone = clone_root.join("hub");
    // Clone the branch the hub's skills actually live on. Falls back to the
    // default branch if that branch does not exist yet (mirrors pusher.rs).
    let clone_branch = remote_cfg.direct_branch.as_deref();
    match git.clone(&remote_cfg.url, &hub_clone, clone_branch) {
        Ok(()) => {}
        Err(_) if clone_branch.is_some() => {
            let _ = std::fs::remove_dir_all(&hub_clone);
            git.clone(&remote_cfg.url, &hub_clone, None)?;
        }
        Err(e) => return Err(e),
    }

    let registry = build_from_hub_clone(&hub_clone, &remote_name)?;
    let found = registry.skills.len();

    let body = serde_json::to_string_pretty(&registry).map_err(|e| QuayError::InvalidRegistry {
        reason: format!("serialise registry: {}", e),
    })?;
    std::fs::write(hub_clone.join("registry.json"), body).map_err(|source| QuayError::Io {
        path: hub_clone.display().to_string(),
        source,
    })?;
    eprintln!(
        "rebuild-registry: indexed {} skill{}",
        found,
        if found == 1 { "" } else { "s" }
    );

    git.add_all(&hub_clone)?;
    let (author_name, author_email) = author_identity(&cfg)?;
    let did_commit = git.commit(
        &hub_clone,
        "rebuild registry.json from disk via quay rebuild-registry",
        &author_name,
        &author_email,
    )?;
    if !did_commit {
        eprintln!("rebuild-registry: registry.json already matches disk; nothing to push.");
        let _ = std::fs::remove_dir_all(&clone_root);
        if json {
            println!("{}", serde_json::json!({"status": "noop", "skills": found}));
        }
        return Ok(());
    }

    match effective_mode {
        PushMode::Direct => {
            // Push to the configured direct_branch; if the clone fell back to
            // the default branch (branch absent on remote), create it here.
            let push_branch = match remote_cfg.direct_branch.as_deref() {
                Some(b) => {
                    if git.current_branch(&hub_clone)? != b {
                        git.checkout_new_branch(&hub_clone, b)?;
                    }
                    b.to_string()
                }
                None => git.current_branch(&hub_clone)?,
            };
            git.push(&hub_clone, "origin", &push_branch).map_err(|e| {
                QuayError::ConfigValidation(format!(
                    "direct push to '{}' failed: {}; if the branch is protected, use --push-mode pr",
                    push_branch, e
                ))
            })?;
            let sha = git.head_sha(&hub_clone).unwrap_or_default();
            let _ = std::fs::remove_dir_all(&clone_root);
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "status": "pushed",
                        "mode": "direct",
                        "branch": push_branch,
                        "commit_sha": sha,
                        "skills": found,
                    })
                );
            } else {
                eprintln!(
                    "rebuild-registry: pushed direct to {} at {}",
                    push_branch, sha
                );
            }
        }
        PushMode::Pr => {
            let branch = format!("quay/rebuild-registry-{}", now_unix_ts());
            git.checkout_new_branch(&hub_clone, &branch)?;
            git.push(&hub_clone, "origin", &branch)?;
            let opener = provider_for_remote(&remote_cfg.url, remote_cfg.provider);
            let pr = opener.open_pr(
                &hub_clone,
                &branch,
                "rebuild registry.json from disk truth",
                &format!(
                    "Regenerated by `quay rebuild-registry`.\n\nIndexed {} skill{} from `skills/` on `{}`.",
                    found,
                    if found == 1 { "" } else { "s" },
                    clone_branch.unwrap_or("the default branch")
                ),
            )?;
            let _ = std::fs::remove_dir_all(&clone_root);
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "status": "pr",
                        "mode": "pr",
                        "branch": branch,
                        "pr_url": pr.url,
                        "skills": found,
                    })
                );
            } else {
                eprintln!("rebuild-registry: opened PR {}", pr.url);
            }
        }
    }
    Ok(())
}

fn author_identity(cfg: &Config) -> Result<(String, String)> {
    let name = cfg.user.name.clone().unwrap_or_else(|| "Quay User".into());
    let email = cfg.user.email.clone().ok_or_else(|| {
        QuayError::ConfigValidation(
            "no author email configured; set [user] email in your config".into(),
        )
    })?;
    Ok((name, email))
}

fn now_unix_ts() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
