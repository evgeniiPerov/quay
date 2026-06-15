use quay_core::{
    add_plan::{
        build_plan, build_plan_with_prompt, collision_names, CollisionStrategy, SkillAction,
    },
    apply_all,
    push_log::PushLog,
    reconcile::{
        action::{apply as reconcile_apply, ResolveAction},
        diff::Diff,
        harbor_history::GitHarborHistory,
        reconcile,
        verdict::{SemverRel, Verdict},
    },
    scanner::scan_local,
    CloneFetcher, Config, MirrorAction, QuayError, RegistryFetcher, SkillFileFetcher, SkillManager,
};
use std::collections::HashMap;
use std::io::IsTerminal as _;
use std::path::Path;

/// Install a single skill from a git URL (GitHub or arbitrary git) without
/// requiring a pre-configured remote in `.quay/config.toml`.
///
/// Used by `quay lock --sync` to install missing skills that have a
/// `sourceType` of `github` or `git`.  A synthetic one-remote `Config` is
/// built from `hub_url` so the normal `SkillManager` machinery can be reused.
///
/// Returns `Ok(())` on success or a descriptive error.
pub fn install_from_url(
    skill_name: &str,
    hub_url: &str,
    project: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    // Build a minimal Config with a single synthetic remote pointing at hub_url.
    let mut cfg = quay_core::Config::default();
    cfg.remotes.insert(
        "_sync_remote".to_string(),
        quay_core::RemoteConfig {
            url: hub_url.to_string(),
            default: true,
            provider: None,
            push_mode: quay_core::PushMode::default(),
            direct_branch: None,
        },
    );

    let f = CloneFetcher::new();
    let mgr = quay_core::SkillManager::new(&cfg, &f, &f, project.to_path_buf());
    mgr.add(skill_name, Some("_sync_remote"))
        .map_err(|e| -> Box<dyn std::error::Error> { e.into() })
}

/// Per-URL cache of harbor clones for the batch `PromptEach` flow.
///
/// Clones each remote URL at most once; a failed clone is recorded as `None`
/// so the warning is only printed once per URL.
struct HarborCache {
    map: HashMap<String, Option<GitHarborHistory>>,
}

impl HarborCache {
    fn new() -> Self {
        Self {
            map: HashMap::new(),
        }
    }

    /// Return a reference to the cached harbor for `url` (cloning on first
    /// access).  Returns `None` when the clone failed; the error is printed
    /// once and then the absence is cached so subsequent calls are silent.
    fn get_or_clone(&mut self, url: &str, branch: Option<&str>) -> Option<&GitHarborHistory> {
        if !self.map.contains_key(url) {
            match GitHarborHistory::clone_harbor(url, branch) {
                Ok(h) => {
                    self.map.insert(url.to_string(), Some(h));
                }
                Err(e) => {
                    eprintln!("warning: could not clone harbor '{}': {}", url, e);
                    self.map.insert(url.to_string(), None);
                }
            }
        }
        self.map.get(url).and_then(|opt| opt.as_ref())
    }
}

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
                    // One CloneFetcher + one SkillManager for all resolve() calls in the closure.
                    // Constructing SkillManager here (not inside the closure) ensures
                    // check_legacy_lockfile runs at most once, not once per colliding skill.
                    let resolve_fetcher = CloneFetcher::new();
                    let mgr = SkillManager::new(
                        &cfg,
                        &resolve_fetcher,
                        &resolve_fetcher,
                        project.to_path_buf(),
                    );
                    let mut harbor_cache = HarborCache::new();
                    build_plan_with_prompt(&pick_names, &locals, |name, _is_modified| {
                        // Resolve which remote + registry entry owns this skill.
                        let (resolved_remote, _registry, entry) = match mgr
                            .resolve(name, Some(remote_name.as_str()))
                        {
                            Ok(t) => t,
                            Err(e) => {
                                eprintln!("warning: could not resolve '{}': {}; skipping", name, e);
                                return SkillAction::Skip;
                            }
                        };

                        // Find the local skill.
                        let local = match locals.iter().find(|l| l.meta.name == name) {
                            Some(l) => l,
                            None => {
                                eprintln!(
                                    "warning: '{}' reported as collision but not found locally; skipping",
                                    name
                                );
                                return SkillAction::Skip;
                            }
                        };

                        // Read local bytes.
                        let local_bytes = match std::fs::read(local.canonical_path()) {
                            Ok(b) => b,
                            Err(e) => {
                                eprintln!(
                                    "warning: could not read local '{}': {}; skipping",
                                    name, e
                                );
                                return SkillAction::Skip;
                            }
                        };

                        // Look up remote config for URL + branch.
                        let remote_cfg = match cfg.remotes.get(&resolved_remote) {
                            Some(r) => r,
                            None => {
                                eprintln!(
                                    "warning: remote '{}' not in config for '{}'; skipping",
                                    resolved_remote, name
                                );
                                return SkillAction::Skip;
                            }
                        };

                        // Get or clone the harbor (one clone per URL for the whole batch).
                        let harbor = match harbor_cache
                            .get_or_clone(&remote_cfg.url, remote_cfg.direct_branch.as_deref())
                        {
                            Some(h) => h,
                            None => {
                                eprintln!("warning: could not reconcile '{}'; skipping", name);
                                return SkillAction::Skip;
                            }
                        };

                        // Reconcile.
                        let skill_path = format!("{}/SKILL.md", entry.path);
                        let report = match reconcile(
                            &local_bytes,
                            local.canonical_sha256(),
                            harbor,
                            &skill_path,
                            &entry.version,
                            &local.meta.version,
                        ) {
                            Ok(r) => r,
                            Err(e) => {
                                eprintln!(
                                    "warning: reconcile failed for '{}': {}; skipping",
                                    name, e
                                );
                                return SkillAction::Skip;
                            }
                        };

                        // Identical — nothing to do.
                        if report.verdict == Verdict::Identical {
                            println!("{}: identical to harbor — nothing to do.", name);
                            return SkillAction::Skip;
                        }

                        // Print verdict + diff.
                        print_verdict_line(name, &report.verdict, report.semver);
                        print_diff(&report.diff);

                        // Prompt user.
                        let action = match prompt_resolve(report.absent_on_head) {
                            Ok(a) => a,
                            Err(e) => {
                                eprintln!("warning: prompt failed for '{}': {}; skipping", name, e);
                                return SkillAction::Skip;
                            }
                        };

                        // Map to SkillAction.
                        // Replace → UpdateForce: the post-plan executor calls run_with with
                        //   do_force=true which overwrites the local file from remote.
                        // Keep / Skip → SkillAction::Skip: local file is left untouched.
                        match action {
                            ResolveAction::Replace => SkillAction::UpdateForce,
                            ResolveAction::Keep | ResolveAction::Skip => SkillAction::Skip,
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
                    false,
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

    // Keep the lockfile current if this project uses one.
    if project.join(quay_core::lock::LOCKFILE_NAME).exists() {
        crate::commands::lock::regenerate(project)?;
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

    let items = [
        "Replace all from harbor",
        "Keep all local",
        "Decide per skill",
    ];
    let idx = dialoguer::Select::new()
        .with_prompt("What should we do with the existing ones?")
        .items(items)
        .default(0)
        .interact()?;

    Ok(match idx {
        0 => CollisionStrategy::UpdateAll,
        1 => CollisionStrategy::SkipAll,
        _ => CollisionStrategy::PromptEach,
    })
}

#[allow(clippy::too_many_arguments)]
pub fn run(
    skill: &str,
    remote: Option<&str>,
    force: bool,
    no_diff: bool,
    profile: Option<&str>,
    project: &Path,
    user_config: Option<&Path>,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let project_config = project.join(".quay/config.toml");
    let cfg = Config::load_resolved(user_config, Some(&project_config), profile)?;

    crate::commands::ensure_remotes_configured(&cfg)?;

    let f = CloneFetcher::new();
    run_with(&cfg, &f, &f, skill, remote, force, no_diff, project, json)?;

    // Keep the lockfile current if this project uses one.
    if project.join(quay_core::lock::LOCKFILE_NAME).exists() {
        crate::commands::lock::regenerate(project)?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn run_with<R: RegistryFetcher, F: SkillFileFetcher>(
    cfg: &Config,
    reg_fetcher: &R,
    file_fetcher: &F,
    skill: &str,
    remote: Option<&str>,
    force: bool,
    no_diff: bool,
    project: &Path,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let mgr = SkillManager::new(cfg, reg_fetcher, file_fetcher, project.to_path_buf());
    if force {
        // Fast path: unconditional overwrite, no reconcile, no harbor clone.
        mgr.add_with_force(skill, remote, true)?;
    } else {
        match mgr.add(skill, remote) {
            Ok(()) => {
                // Fresh install — nothing to reconcile.
            }
            Err(QuayError::AlreadyExists(_)) => {
                // Skill already exists locally — attempt reconcile before erroring.
                handle_collision(
                    cfg,
                    reg_fetcher,
                    file_fetcher,
                    skill,
                    remote,
                    no_diff,
                    project,
                )?;
                // handle_collision either returns Ok (identical/resolved) or propagates Err.
                // If it returned Ok we skip the rest of the install block.
                if json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "action": "reconciled",
                            "skill": skill,
                        }))?
                    );
                }
                apply_mirrors_after_install(cfg, project, skill, json);
                return Ok(());
            }
            Err(e) => return Err(e.into()),
        }
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

/// Handle a single-skill `AlreadyExists` collision via the reconcile engine.
///
/// Returns `Ok(())` when the collision is resolved (Identical / Replace / Keep /
/// Skip) or propagates the original `AlreadyExists`-style error when the caller
/// should treat this as a blocking collision (non-TTY without resolution).
fn handle_collision<R: RegistryFetcher, F: SkillFileFetcher>(
    cfg: &Config,
    reg_fetcher: &R,
    file_fetcher: &F,
    skill: &str,
    pinned_remote: Option<&str>,
    no_diff: bool,
    project: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    // Resolve which remote + registry entry owns this skill.
    let mgr = SkillManager::new(cfg, reg_fetcher, file_fetcher, project.to_path_buf());
    let (remote_name, _registry, entry) = mgr
        .resolve(skill, pinned_remote)
        .map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
    let remote_cfg = &cfg.remotes[&remote_name];

    // Find the local skill for sha + canonical path.
    let config_dir = crate::config_io::default_config_dir();
    let log =
        PushLog::load(config_dir.as_deref().unwrap_or(project), Some(project)).unwrap_or_default();
    let locals = scan_local(project, &log);
    let local = locals
        .iter()
        .find(|s| s.meta.name == skill)
        .ok_or_else(|| format!("collision reported but skill '{}' not found locally", skill))?;

    let local_bytes = std::fs::read(local.canonical_path())?;
    let local_sha = local.canonical_sha256();
    let skill_path = format!("{}/SKILL.md", entry.path);

    // Try to clone the harbor and reconcile.
    let harbor_result =
        GitHarborHistory::clone_harbor(&remote_cfg.url, remote_cfg.direct_branch.as_deref());

    let harbor = match harbor_result {
        Err(e) => {
            // Harbor unreachable — warn and fall through to original collision error.
            eprintln!(
                "warning: could not reach harbor to compare {}: {}",
                skill, e
            );
            return Err(
                QuayError::AlreadyExists(local.canonical_path().display().to_string()).into(),
            );
        }
        Ok(h) => h,
    };

    let report = reconcile(
        &local_bytes,
        local_sha,
        &harbor,
        &skill_path,
        &entry.version,
        &local.meta.version,
    )?;

    if report.verdict == Verdict::Identical {
        println!("{}: identical to harbor — nothing to do.", skill);
        return Ok(());
    }

    // Print verdict line.
    print_verdict_line(skill, &report.verdict, report.semver);

    // Optionally print diff.
    if !no_diff {
        print_diff(&report.diff);
    }

    // Prompt or non-TTY error.
    let action = if std::io::stdin().is_terminal() {
        prompt_resolve(report.absent_on_head)?
    } else {
        eprintln!(
            "{}: skill differs from harbor. Re-run with --force to overwrite, or interactively to reconcile.",
            skill
        );
        return Err(QuayError::AlreadyExists(local.canonical_path().display().to_string()).into());
    };

    reconcile_apply(action, local.canonical_path(), &report.head_bytes)?;

    match action {
        ResolveAction::Replace => println!("{}: replaced with harbor copy.", skill),
        ResolveAction::Keep => println!("{}: kept local copy.", skill),
        ResolveAction::Skip => println!("{}: skipped.", skill),
    }

    Ok(())
}

/// Print a human-readable verdict line for a colliding skill.
fn print_verdict_line(name: &str, verdict: &Verdict, semver: SemverRel) {
    let verdict_str = match verdict {
        Verdict::Identical => "identical".to_string(),
        Verdict::HubNewer {
            commits_ahead,
            last_commit_date,
            ..
        } => format!(
            "HARBOR NEWER — {} commit(s) ahead, last {}",
            commits_ahead, last_commit_date
        ),
        Verdict::LocalAheadOrDiverged { .. } => "LOCAL diverged from harbor".to_string(),
        Verdict::ChangedUnknownDirection { local_edited } => {
            if *local_edited {
                "CHANGED — differs from harbor (direction unknown, local edits present)".to_string()
            } else {
                "CHANGED — differs from harbor (direction unknown)".to_string()
            }
        }
    };
    println!("{}: {}  [semver: {:?}]", name, verdict_str, semver);
}

/// Print the diff body.
fn print_diff(diff: &Diff) {
    match diff {
        Diff::Text(s) => print!("{}", s),
        Diff::Binary {
            hub_bytes,
            local_bytes,
        } => println!(
            "(binary/non-UTF8: {} bytes harbor vs {} local)",
            hub_bytes, local_bytes
        ),
    }
}

/// Prompt the user to choose a resolve action via `dialoguer::Select`.
///
/// When `absent_on_head` is true, Replace is omitted (nothing on harbor HEAD).
fn prompt_resolve(absent_on_head: bool) -> Result<ResolveAction, Box<dyn std::error::Error>> {
    if absent_on_head {
        let items = ["Keep local", "Skip"];
        let idx = dialoguer::Select::new()
            .with_prompt("How should this collision be resolved?")
            .items(items)
            .default(0)
            .interact()?;
        Ok(if idx == 0 {
            ResolveAction::Keep
        } else {
            ResolveAction::Skip
        })
    } else {
        let items = ["Replace with harbor", "Keep local", "Skip"];
        let idx = dialoguer::Select::new()
            .with_prompt("How should this collision be resolved?")
            .items(items)
            .default(0)
            .interact()?;
        Ok(match idx {
            0 => ResolveAction::Replace,
            1 => ResolveAction::Keep,
            _ => ResolveAction::Skip,
        })
    }
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
