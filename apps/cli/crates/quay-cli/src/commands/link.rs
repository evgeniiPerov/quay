//! Implementation of `quay link`.

use crate::args::LinkAction;
use crate::commands::interactive::is_tty;
use crate::config_io::{read_project_file, write_project_file};
use quay_core::{
    apply_all, check, classify, discover_roots, reconcile, Config, InstallConfig, MirrorAction,
    MirrorConfig, MirrorDrift, MirrorState, MirrorStrategy, ReconcileReport,
};
use serde_json::json;
use std::path::Path;

pub fn run(
    action: Option<LinkAction>,
    force: bool,
    project: &Path,
    user_config: Option<&Path>,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    match action {
        None => apply(project, user_config, force, json),
        Some(LinkAction::Check) => check_cmd(project, user_config, json),
        Some(LinkAction::Add { path, strategy }) => add_mirror(&path, &strategy, project, json),
        Some(LinkAction::Remove { path }) => remove_mirror(&path, project, json),
    }
}

fn list_installed_skills(install: &InstallConfig, project: &Path) -> Vec<String> {
    let canonical = project.join(&install.canonical);
    if !canonical.exists() {
        return Vec::new();
    }
    let mut names = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&canonical) {
        for entry in entries.flatten() {
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                if let Some(name) = entry.file_name().to_str() {
                    names.push(name.to_string());
                }
            }
        }
    }
    names.sort();
    names
}

fn apply(
    project: &Path,
    user_config: Option<&Path>,
    force: bool,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let project_config = project.join(".quay/config.toml");
    let mut cfg = Config::load_resolved(user_config, Some(&project_config), None)?;
    let skills = list_installed_skills(&cfg.install, project);

    let mut report = reconcile(&cfg.install, project, &skills, force)?;

    // One-time opt-in: adoptable dirs found, choice not yet recorded, interactive.
    if !report.needs_optin.is_empty() && cfg.install.auto_link.is_none() && !json && is_tty() {
        let yes = dialoguer::Confirm::new()
            .with_prompt(format!(
                "Found {} unmanaged tool dir(s) matching canonical. Adopt them into canonical (symlink on most platforms)?",
                report.needs_optin.len()
            ))
            .default(true)
            .interact()?;
        cfg.install.auto_link = Some(yes);
        if yes {
            report = reconcile(&cfg.install, project, &skills, force)?;
        }
        persist_auto_link(project, yes)?;
    }

    register_adopted_mirrors(project, &report)?;

    render_report(&report, json)?;

    // needs_optin only fails the command while the choice is undecided (None) —
    // an explicit opt-out (Some(false)) means "report only", not an error.
    let unresolved_optin = if cfg.install.auto_link == Some(false) {
        0
    } else {
        report.needs_optin.len()
    };
    let unresolved = report.diverged.len() + unresolved_optin;
    if unresolved > 0 {
        return Err(format!("{} mirror(s) need attention", unresolved).into());
    }
    Ok(())
}

/// Write `install.auto_link = <yes>` back to the project config file.
fn persist_auto_link(project: &Path, yes: bool) -> Result<(), Box<dyn std::error::Error>> {
    let mut file = read_project_file(project)?;
    file.install.auto_link = Some(yes);
    write_project_file(project, &file)?;
    Ok(())
}

/// Register any newly-adopted discovered root (e.g. `.codex/skills`) in config
/// so future commands track it explicitly.
fn register_adopted_mirrors(
    project: &Path,
    report: &ReconcileReport,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut file = read_project_file(project)?;
    let mut changed = false;
    for (_, path, action) in &report.actions {
        if !matches!(action, MirrorAction::Adopted { .. }) {
            continue;
        }
        // `path` is `<root>/<skill>`; the mirror root is its parent.
        if let Some(root) = path.parent() {
            let rel = root.strip_prefix(project).unwrap_or(root).to_path_buf();
            if !file.install.mirrors.iter().any(|m| m.path == rel) {
                file.install.mirrors.push(MirrorConfig {
                    path: rel,
                    strategy: MirrorStrategy::Auto,
                });
                changed = true;
            }
        }
    }
    if changed {
        write_project_file(project, &file)?;
    }
    Ok(())
}

fn render_report(report: &ReconcileReport, json: bool) -> Result<(), Box<dyn std::error::Error>> {
    if json {
        let payload = json!({
            "actions": report.actions.iter().map(|(skill, path, action)| {
                let (kind, strat) = match action {
                    MirrorAction::Created { strategy, .. } => ("created", Some(strategy)),
                    MirrorAction::Replaced { strategy, .. } => ("replaced", Some(strategy)),
                    MirrorAction::Adopted { strategy, .. } => ("adopted", Some(strategy)),
                    MirrorAction::NoOp => ("noop", None),
                };
                json!({
                    "skill": skill,
                    "path": path.display().to_string(),
                    "action": kind,
                    "strategy": strat.map(|s| format!("{:?}", s).to_lowercase()),
                })
            }).collect::<Vec<_>>(),
            "diverged": report.diverged.iter().map(|d| json!({
                "skill": d.skill, "path": d.mirror_path.display().to_string(), "reason": d.reason
            })).collect::<Vec<_>>(),
            "needs_optin": report.needs_optin.iter().map(|(s, p)| json!({
                "skill": s, "path": p.display().to_string()
            })).collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
    } else {
        if report.actions.is_empty() && report.diverged.is_empty() && report.needs_optin.is_empty()
        {
            println!("ok: all mirrors intact");
        }
        for (skill, path, action) in &report.actions {
            match action {
                MirrorAction::Created { strategy, .. } => {
                    println!("created  {} -> {} ({:?})", skill, path.display(), strategy);
                }
                MirrorAction::Replaced { strategy, .. } => {
                    println!("replaced {} -> {} ({:?})", skill, path.display(), strategy);
                }
                MirrorAction::Adopted { strategy, .. } => {
                    println!("adopted  {} -> {} ({:?})", skill, path.display(), strategy);
                }
                MirrorAction::NoOp => {}
            }
        }
        for d in &report.diverged {
            eprintln!(
                "diverged {} at {}: {}",
                d.skill,
                d.mirror_path.display(),
                d.reason
            );
            eprintln!("  keep it: copy your edit to the canonical skill, then re-run");
            eprintln!("  discard: quay link --force");
        }
        for (skill, path) in &report.needs_optin {
            eprintln!(
                "unmanaged {} at {} (matches canonical) — enable adopt to symlink it",
                skill,
                path.display()
            );
        }
    }
    Ok(())
}

fn check_cmd(
    project: &Path,
    user_config: Option<&Path>,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let project_config = project.join(".quay/config.toml");
    let cfg = Config::load_resolved(user_config, Some(&project_config), None)?;
    let skills = list_installed_skills(&cfg.install, project);
    // Configured-mirror drift (missing / symlink target mismatch / content drift).
    let mut drift = check(&cfg.install, project, &skills)?;
    // Discovery-driven divergence / unmanaged dirs not in `install.mirrors`.
    // Read-only: classify() never writes, unlike reconcile()/apply_one().
    let roots = discover_roots(&cfg.install, project);
    let canonical_root = project.join(&cfg.install.canonical);
    for name in &skills {
        let canonical_skill = canonical_root.join(name);
        if !canonical_skill.exists() {
            continue;
        }
        for (rel, _strategy) in &roots {
            let target = project.join(rel).join(name);
            match classify(&target, &canonical_skill)? {
                MirrorState::Diverged { reason } | MirrorState::Conflict { reason } => {
                    drift.push(MirrorDrift {
                        skill: name.clone(),
                        mirror_path: target,
                        reason,
                    });
                }
                MirrorState::Adoptable => {
                    // When the user has opted out (`auto_link = false`), an
                    // adoptable dir is an accepted state, not drift.
                    if cfg.install.auto_link != Some(false) {
                        drift.push(MirrorDrift {
                            skill: name.clone(),
                            mirror_path: target,
                            reason: "unmanaged directory; run `quay link` to adopt".into(),
                        });
                    }
                }
                // Missing: a discovered root simply not mirroring this skill is
                // normal, not drift (configured-mirror Missing is already
                // reported by `check` above). Correct: nothing to report.
                MirrorState::Missing | MirrorState::Correct => {}
            }
        }
    }
    drift.sort_by(|a, b| {
        (a.skill.clone(), a.mirror_path.clone()).cmp(&(b.skill.clone(), b.mirror_path.clone()))
    });
    drift.dedup_by(|a, b| a.skill == b.skill && a.mirror_path == b.mirror_path);

    if drift.is_empty() {
        if json {
            println!("{}", serde_json::to_string_pretty(&json!({"drift": []}))?);
        } else {
            println!("ok: all mirrors intact");
        }
        Ok(())
    } else {
        if json {
            let payload: Vec<_> = drift
                .iter()
                .map(|d| {
                    json!({
                        "skill": d.skill,
                        "path": d.mirror_path.display().to_string(),
                        "reason": d.reason,
                    })
                })
                .collect();
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({"drift": payload}))?
            );
        } else {
            for d in &drift {
                eprintln!(
                    "drift: {} at {}: {}",
                    d.skill,
                    d.mirror_path.display(),
                    d.reason
                );
            }
        }
        Err(format!("{} mirror(s) out of sync", drift.len()).into())
    }
}

fn add_mirror(
    path: &Path,
    strategy_str: &str,
    project: &Path,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let strategy = match strategy_str {
        "auto" => MirrorStrategy::Auto,
        "symlink" => MirrorStrategy::Symlink,
        "junction" => MirrorStrategy::Junction,
        "copy" => MirrorStrategy::Copy,
        other => {
            return Err(format!(
                "invalid --strategy '{}': expected auto|symlink|junction|copy",
                other
            )
            .into())
        }
    };
    let mut file = read_project_file(project)?;
    if file.install.mirrors.iter().any(|m| m.path == path) {
        return Err(format!("mirror already configured: {}", path.display()).into());
    }
    file.install.mirrors.push(MirrorConfig {
        path: path.to_path_buf(),
        strategy,
    });
    write_project_file(project, &file)?;

    let cfg = Config::load_resolved(None, Some(&project.join(".quay/config.toml")), None)?;
    for name in list_installed_skills(&cfg.install, project) {
        apply_all(&cfg.install, project, &name, false)?;
    }

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "added_mirror": path.display().to_string(),
                "strategy": strategy_str,
            }))?
        );
    } else {
        println!("added mirror {} ({})", path.display(), strategy_str);
    }
    Ok(())
}

fn remove_mirror(
    path: &Path,
    project: &Path,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut file = read_project_file(project)?;
    let before = file.install.mirrors.len();
    file.install.mirrors.retain(|m| m.path != path);
    if file.install.mirrors.len() == before {
        return Err(format!("no mirror configured at {}", path.display()).into());
    }
    write_project_file(project, &file)?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({"removed_mirror": path.display().to_string()}))?
        );
    } else {
        println!("removed mirror {}", path.display());
        println!(
            "  note: existing files at {} were not deleted",
            path.display()
        );
    }
    Ok(())
}
