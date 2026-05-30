//! `quay remove` — delete a skill from all local mirror roots.
//!
//! With `--everywhere`, also pushes a deletion commit to each configured
//! remote that publishes the skill (requires git + push access).
//! With `-i` / `--interactive`, opens a multi-select picker over local skills.

use quay_core::{CloneFetcher, Config, SkillManager};
use serde_json::json;
use std::path::Path;

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

/// Remove one or more local skills selected interactively via `dialoguer::MultiSelect`.
///
/// When `everywhere` is `true`, prompts for confirmation then pushes a deletion
/// commit to every remote that publishes each picked skill.
///
/// Returns `Err(InteractiveUnavailable)` immediately when stdin is not a TTY.
#[allow(clippy::too_many_arguments)]
pub fn run_interactive(
    everywhere: bool,
    profile: Option<&str>,
    project: &Path,
    user_config: Option<&Path>,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let project_config = project.join(".quay/config.toml");
    let cfg = Config::load_resolved(user_config, Some(&project_config), profile)?;

    // Check TTY immediately so the caller gets a clear error even if the skill
    // list is empty — consistent with add/push interactive paths.
    if !crate::commands::interactive::is_tty() {
        return Err(Box::new(
            crate::commands::interactive::InteractiveUnavailable,
        ));
    }

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
        "Select skills to remove (Space to toggle, Enter to confirm)",
        &skills,
        |s| format!("{} v{} ({:?})", s.meta.name, s.meta.version, s.status),
    )?;

    if picks.is_empty() {
        println!("(nothing selected)");
        return Ok(());
    }

    if everywhere && !picks.is_empty() {
        let n = picks.len();
        let confirmed = dialoguer::Confirm::new()
            .with_prompt(format!("delete {n} skill(s) everywhere? y/N"))
            .default(false)
            .interact()?;
        if !confirmed {
            return Ok(());
        }
    }

    let f = CloneFetcher::new();
    let mgr = SkillManager::new(&cfg, &f, &f, project.to_path_buf());

    for idx in &picks {
        let skill_name = &skills[*idx].meta.name;
        mgr.remove(skill_name)?;
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({"action": "removed", "skill": skill_name}))?
            );
        } else {
            println!("removed {skill_name}");
        }
        if everywhere {
            remove_from_remotes(skill_name, &cfg, project, profile, user_config, json)?;
        }
    }

    Ok(())
}

pub fn run(
    skill: &str,
    everywhere: bool,
    profile: Option<&str>,
    project: &Path,
    user_config: Option<&Path>,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let project_config = project.join(".quay/config.toml");
    let cfg = Config::load_resolved(user_config, Some(&project_config), profile)?;
    // remove only touches the filesystem; we still need the trait bounds
    // satisfied so we pass a fetcher that will not be called.
    let f = CloneFetcher::new();
    let mgr = SkillManager::new(&cfg, &f, &f, project.to_path_buf());
    mgr.remove(skill)?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({"action": "removed", "skill": skill}))?
        );
    } else {
        println!("removed {}", skill);
    }

    if everywhere {
        // Push a deletion commit to each configured remote.
        remove_from_remotes(skill, &cfg, project, profile, user_config, json)?;
    }

    Ok(())
}

/// Push a deletion commit to each configured remote that publishes `skill`.
///
/// Reuses the pusher's clone + commit + push pipeline with a "remove file" branch.
fn remove_from_remotes(
    skill: &str,
    cfg: &Config,
    _project: &Path,
    _profile: Option<&str>,
    _user_config: Option<&Path>,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    use quay_core::GitShellClient;

    let git = GitShellClient;

    // Determine author identity from config.
    let author_name = cfg.user.name.clone().unwrap_or_else(|| "Quay User".into());
    let author_email = cfg
        .user
        .email
        .clone()
        .ok_or("no author email in config — run `quay profile add` first")?;

    let clone_root = std::env::temp_dir().join(format!("quay-delete-{skill}"));
    if clone_root.exists() {
        std::fs::remove_dir_all(&clone_root)?;
    }
    std::fs::create_dir_all(&clone_root)?;

    let mut deleted_remotes: Vec<String> = Vec::new();
    let mut errors: Vec<String> = Vec::new();

    for (remote_name, remote_cfg) in &cfg.remotes {
        let hub_clone = clone_root.join(format!("hub-{remote_name}"));

        // Clone the hub.
        if let Err(e) = git.clone(&remote_cfg.url, &hub_clone, None) {
            errors.push(format!("{remote_name}: clone failed: {e}"));
            continue;
        }

        // Check if the skill exists on this remote.
        let skill_dir = hub_clone.join("skills").join(skill);
        let skill_md = skill_dir.join("SKILL.md");
        if !skill_md.exists() {
            // Not on this remote — skip silently.
            continue;
        }

        // Remove the skill directory.
        if let Err(e) = std::fs::remove_dir_all(&skill_dir) {
            errors.push(format!("{remote_name}: rm failed: {e}"));
            continue;
        }

        // Update registry.json — remove the skill entry.
        let registry_path = hub_clone.join("registry.json");
        if registry_path.exists() {
            if let Ok(text) = std::fs::read_to_string(&registry_path) {
                if let Ok(mut registry) = quay_core::Registry::parse(&text) {
                    registry.skills.remove(skill);
                    // generated_at update is best-effort; leave as-is to avoid
                    // pulling in chrono directly in quay-cli.
                    if let Ok(body) = serde_json::to_string_pretty(&registry) {
                        let _ = std::fs::write(&registry_path, body);
                    }
                }
            }
        }

        // Commit + push.
        let commit_msg = format!("remove skill {skill}");
        if let Err(e) = git.add_all(&hub_clone) {
            errors.push(format!("{remote_name}: git add failed: {e}"));
            continue;
        }
        match git.commit(&hub_clone, &commit_msg, &author_name, &author_email) {
            Ok(false) => {
                // Nothing to commit — skill wasn't tracked by git yet.
                continue;
            }
            Ok(true) => {}
            Err(e) => {
                errors.push(format!("{remote_name}: commit failed: {e}"));
                continue;
            }
        }

        // Push using the remote's configured push mode (direct to default branch).
        use quay_core::GitClient;
        match git.current_branch(&hub_clone) {
            Ok(branch) => {
                if let Err(e) = git.push(&hub_clone, "origin", &branch) {
                    errors.push(format!("{remote_name}: push failed: {e}"));
                } else {
                    deleted_remotes.push(remote_name.clone());
                }
            }
            Err(e) => {
                errors.push(format!("{remote_name}: get branch failed: {e}"));
            }
        }
    }

    // Clean up.
    let _ = std::fs::remove_dir_all(&clone_root);

    if !json {
        for r in &deleted_remotes {
            println!("  deleted from remote: {r}");
        }
        for e in &errors {
            eprintln!("  warning: {e}");
        }
    }

    // Surface errors as a non-fatal warning (local removal already succeeded).
    if !errors.is_empty() && deleted_remotes.is_empty() {
        eprintln!("warning: local removal succeeded but remote deletion failed for all remotes");
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
