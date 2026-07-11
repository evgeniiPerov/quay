//! `quay remove` — delete a skill locally, from the hub, or both.
//!
//! Default: delete from all local mirror roots only.
//! With `--remote`: delete from the hub (active profile's default remote) only,
//! keeping the local copy (requires git + push access).
//! With `--everywhere`: delete both locally and from the hub.
//! With `-i` / `--interactive`: opens a 3-way scope picker (Local / Remote /
//! Everywhere); a bare `--remote` jumps straight to the hub picker.

use quay_core::{CloneFetcher, Config, SkillManager};
use serde_json::json;
use std::path::Path;

/// Removes a directory when dropped, so a temp clone is cleaned up on every
/// exit path (including `?` early returns and panics).
struct TempDirGuard(std::path::PathBuf);

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Where a `quay remove` should delete the skill.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoveScope {
    /// Delete local mirror dirs only; hub untouched.
    Local,
    /// Delete from the default remote's hub only; local copy untouched.
    Remote,
    /// Delete both local and hub.
    Everywhere,
}

impl RemoveScope {
    /// Resolve scope from the two boolean flags. `--remote` and `--everywhere`
    /// are mutually exclusive at the clap layer, so at most one is true here.
    pub fn from_flags(remote: bool, everywhere: bool) -> Self {
        if everywhere {
            Self::Everywhere
        } else if remote {
            Self::Remote
        } else {
            Self::Local
        }
    }
}

/// Interactive remove. `forced_scope = None` shows a 3-way scope menu;
/// `Some(scope)` skips the menu (used by the bare `--remote` shortcut).
pub fn run_interactive(
    forced_scope: Option<RemoveScope>,
    profile: Option<&str>,
    project: &Path,
    user_config: Option<&Path>,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    use quay_core::fetcher::RegistryFetcher;

    let project_config = project.join(".quay/config.toml");
    let cfg = Config::load_resolved(user_config, Some(&project_config), profile)?;

    if !crate::commands::interactive::is_tty() {
        return Err(Box::new(
            crate::commands::interactive::InteractiveUnavailable,
        ));
    }

    let scope = match forced_scope {
        Some(s) => s,
        None => {
            let choice = dialoguer::Select::new()
                .with_prompt("Remove scope")
                .items(["Local", "Remote (hub)", "Everywhere"])
                .default(0)
                .interact()?;
            match choice {
                1 => RemoveScope::Remote,
                2 => RemoveScope::Everywhere,
                _ => RemoveScope::Local,
            }
        }
    };

    let f = CloneFetcher::new();
    let mgr = SkillManager::new(&cfg, &f, &f, project.to_path_buf());

    match scope {
        RemoveScope::Local => {
            let push_log = quay_core::push_log::PushLog::load(
                user_config.and_then(|p| p.parent()).unwrap_or(project),
                Some(project),
            )
            .unwrap_or_default();
            let skills = quay_core::scanner::scan_local(project, &push_log);
            if skills.is_empty() {
                println!("(no local skills found)");
                return Ok(());
            }
            let picks = crate::commands::interactive::pick_many(
                "Select skills to remove locally (Space to toggle, Enter to confirm)",
                &skills,
                |s| {
                    format!(
                        "{} {} ({:?})",
                        s.meta.name,
                        s.meta.version_display(),
                        s.status
                    )
                },
            )?;
            if picks.is_empty() {
                println!("(nothing selected)");
                return Ok(());
            }
            for idx in &picks {
                let name = &skills[*idx].meta.name;
                mgr.remove(name)?;
                println!("removed {name}");
            }
        }
        RemoveScope::Remote | RemoveScope::Everywhere => {
            let (remote_name, remote_cfg) = cfg
                .default_remote()
                .ok_or("no default remote configured — add one with `quay remote add`")?;
            let branch_label = remote_cfg
                .direct_branch
                .as_deref()
                .unwrap_or("default branch");
            let fetcher = CloneFetcher::new();
            let registry = match remote_cfg.direct_branch.as_deref() {
                Some(b) => fetcher.fetch_at(&remote_cfg.url, b)?,
                None => fetcher.fetch(&remote_cfg.url)?,
            };
            let names: Vec<String> = registry.skills.keys().cloned().collect();
            if names.is_empty() {
                println!("(no skills on {remote_name})");
                return Ok(());
            }
            let picks = crate::commands::interactive::pick_many(
                &format!("Select skills to remove from {remote_name} ({branch_label})"),
                &names,
                |n| n.clone(),
            )?;
            if picks.is_empty() {
                println!("(nothing selected)");
                return Ok(());
            }
            let n = picks.len();
            let confirmed = dialoguer::Confirm::new()
                .with_prompt(format!(
                    "delete {n} skill(s) from {remote_name} ({branch_label})? y/N"
                ))
                .default(false)
                .interact()?;
            if !confirmed {
                return Ok(());
            }
            for idx in &picks {
                let name = &names[*idx];
                if matches!(scope, RemoveScope::Everywhere) {
                    match mgr.remove(name) {
                        Ok(()) => println!("removed {name}"),
                        Err(quay_core::QuayError::SkillNotFound { .. }) => {}
                        Err(e) => return Err(e.into()),
                    }
                }
                remove_from_default_remote(name, &cfg, json)?;
            }
        }
    }
    // Keep the lockfile current if this project uses one (best-effort).
    crate::commands::lock::regenerate_if_present(project);
    Ok(())
}

/// Non-interactive remove of a single named `skill` at the given [`RemoveScope`]:
/// local mirror dirs, the default remote's hub, or both.
pub fn run(
    skill: &str,
    scope: RemoveScope,
    profile: Option<&str>,
    project: &Path,
    user_config: Option<&Path>,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let project_config = project.join(".quay/config.toml");
    let cfg = Config::load_resolved(user_config, Some(&project_config), profile)?;
    // A fetcher is required to satisfy SkillManager's trait bounds even though
    // local removal never calls it.
    let f = CloneFetcher::new();
    let mgr = SkillManager::new(&cfg, &f, &f, project.to_path_buf());

    let report_local = |json: bool| {
        if json {
            if let Ok(s) =
                serde_json::to_string_pretty(&json!({"action": "removed", "skill": skill}))
            {
                println!("{s}");
            }
        } else {
            println!("removed {skill}");
        }
    };

    match scope {
        RemoveScope::Local => {
            mgr.remove(skill)?;
            report_local(json);
        }
        RemoveScope::Remote => {
            remove_from_default_remote(skill, &cfg, json)?;
        }
        RemoveScope::Everywhere => {
            match mgr.remove(skill) {
                Ok(()) => report_local(json),
                // Not installed locally is fine for everywhere — still remove from hub.
                Err(quay_core::QuayError::SkillNotFound { .. }) => {}
                Err(e) => return Err(e.into()),
            }
            remove_from_default_remote(skill, &cfg, json)?;
        }
    }

    // Keep the lockfile current if this project uses one (best-effort).
    crate::commands::lock::regenerate_if_present(project);
    Ok(())
}

/// Delete `skill` from the active profile's **default remote** only.
///
/// Branch-aware: clones and pushes the remote's configured `direct_branch`
/// (falling back to the default branch if that branch is absent), mirroring
/// `pusher.rs` and `rebuild_registry.rs`. Removes `skills/<skill>`, drops the
/// `registry.json` entry, commits, and pushes. Errors (instead of silently
/// skipping) when the skill is not present on the targeted branch.
fn remove_from_default_remote(
    skill: &str,
    cfg: &Config,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    use quay_core::{GitClient, GitShellClient};

    let git = GitShellClient;

    let (remote_name, remote_cfg) = cfg
        .default_remote()
        .ok_or("no default remote configured — add one with `quay remote add`")?;

    let author_name = cfg.user.name.clone().unwrap_or_else(|| "Quay User".into());
    let author_email = cfg
        .user
        .email
        .clone()
        .ok_or("no author email in config — run `quay profile add` first")?;

    let clone_root =
        std::env::temp_dir().join(format!("quay-delete-{}-{skill}", std::process::id()));
    if clone_root.exists() {
        std::fs::remove_dir_all(&clone_root)?;
    }
    std::fs::create_dir_all(&clone_root)?;
    let hub_clone = clone_root.join("hub");
    // Guard ensures clone_root is removed on every exit path, including `?` returns and panics.
    let _cleanup = TempDirGuard(clone_root.clone());

    // Clone the branch the skills live on; fall back to default branch.
    let clone_branch = remote_cfg.direct_branch.as_deref();
    match git.clone(&remote_cfg.url, &hub_clone, clone_branch) {
        Ok(()) => {}
        Err(_) if clone_branch.is_some() => {
            let _ = std::fs::remove_dir_all(&hub_clone);
            git.clone(&remote_cfg.url, &hub_clone, None)?;
        }
        Err(e) => return Err(e.into()),
    }
    let branch_label = clone_branch.unwrap_or("the default branch");

    // Skill must exist on the targeted branch — explicit error, not silent skip.
    let skill_dir = hub_clone.join("skills").join(skill);
    if !skill_dir.join("SKILL.md").exists() {
        return Err(format!("skill {skill} not found on {remote_name} ({branch_label})").into());
    }

    std::fs::remove_dir_all(&skill_dir)?;

    // Drop the registry entry (best-effort on a malformed registry).
    let registry_path = hub_clone.join("registry.json");
    if let Ok(text) = std::fs::read_to_string(&registry_path) {
        if let Ok(mut registry) = quay_core::Registry::parse(&text) {
            registry.skills.remove(skill);
            if let Ok(body) = serde_json::to_string_pretty(&registry) {
                let _ = std::fs::write(&registry_path, body);
            }
        }
    }

    git.add_all(&hub_clone)?;
    let did_commit = git.commit(
        &hub_clone,
        &format!("remove skill {skill}"),
        &author_name,
        &author_email,
    )?;
    if !did_commit {
        return Err(format!("nothing to remove for {skill} (no change after delete)").into());
    }

    // Push branch-aware: switch to direct_branch if needed.
    let push_branch = match clone_branch {
        Some(b) => {
            if git.current_branch(&hub_clone)? != b {
                git.checkout_new_branch(&hub_clone, b)?;
            }
            b.to_string()
        }
        None => git.current_branch(&hub_clone)?,
    };
    git.push(&hub_clone, "origin", &push_branch).map_err(
        |e| -> Box<dyn std::error::Error> {
            format!(
                "direct push to '{push_branch}' failed: {e}; if the branch is protected, set this remote's push_mode = pr"
            )
            .into()
        },
    )?;

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "action": "removed_remote",
                "skill": skill,
                "remote": remote_name,
                "branch": push_branch,
            }))?
        );
    } else {
        println!("  deleted from remote: {remote_name} ({push_branch})");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remove_local_only_deletes_files() {
        let dir = assert_fs::TempDir::new().unwrap();
        use assert_fs::prelude::*;
        dir.child(".agents/skills/foo/SKILL.md")
            .write_str("---\nname: foo\ndescription: f\n---\n")
            .unwrap();

        let cfg = Config::default();
        let f = CloneFetcher::new();
        let mgr = SkillManager::new(&cfg, &f, &f, dir.path().to_path_buf());
        mgr.remove("foo").unwrap();

        assert!(!dir.path().join(".agents/skills/foo").exists());
    }

    #[test]
    fn remove_not_found_errors() {
        let dir = assert_fs::TempDir::new().unwrap();
        let cfg = Config::default();
        let f = CloneFetcher::new();
        let mgr = SkillManager::new(&cfg, &f, &f, dir.path().to_path_buf());
        let err = mgr.remove("does-not-exist").unwrap_err();
        assert!(
            matches!(err, quay_core::QuayError::SkillNotFound { .. }),
            "expected SkillNotFound, got: {err}"
        );
    }

    #[test]
    fn scope_from_flags_maps_correctly() {
        assert_eq!(RemoveScope::from_flags(false, false), RemoveScope::Local);
        assert_eq!(RemoveScope::from_flags(true, false), RemoveScope::Remote);
        assert_eq!(
            RemoveScope::from_flags(false, true),
            RemoveScope::Everywhere
        );
        // everywhere wins if both somehow set (clap prevents this, defensive).
        assert_eq!(RemoveScope::from_flags(true, true), RemoveScope::Everywhere);
    }
}
