//! Implementation of `quay link`.

use crate::args::LinkAction;
use crate::config_io::{read_project_file, write_project_file};
use quay_core::{
    apply_all, check, Config, InstallConfig, MirrorAction, MirrorConfig, MirrorStrategy,
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
    let cfg = Config::load_resolved(user_config, Some(&project_config), None)?;
    let skills = list_installed_skills(&cfg.install, project);
    let mut actions_out: Vec<(String, Vec<MirrorAction>)> = Vec::new();
    for name in &skills {
        let actions = apply_all(&cfg.install, project, name, force)?;
        actions_out.push((name.clone(), actions));
    }
    if json {
        let payload: Vec<_> = actions_out
            .iter()
            .map(|(s, acts)| {
                json!({
                    "skill": s,
                    "actions": acts.iter().map(|a| match a {
                        MirrorAction::NoOp => json!({"action": "noop"}),
                        MirrorAction::Created { path, strategy } => json!({
                            "action": "created",
                            "path": path.display().to_string(),
                            "strategy": format!("{:?}", strategy).to_lowercase(),
                        }),
                        MirrorAction::Replaced { path, strategy } => json!({
                            "action": "replaced",
                            "path": path.display().to_string(),
                            "strategy": format!("{:?}", strategy).to_lowercase(),
                        }),
                    }).collect::<Vec<_>>(),
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&payload)?);
    } else if cfg.install.mirrors.is_empty() {
        println!("(no mirrors configured)");
    } else if skills.is_empty() {
        println!("(no installed skills to mirror)");
    } else {
        for (name, actions) in &actions_out {
            for action in actions {
                match action {
                    MirrorAction::Created { path, strategy } => {
                        println!("created  {} -> {} ({:?})", name, path.display(), strategy);
                    }
                    MirrorAction::Replaced { path, strategy } => {
                        println!("replaced {} -> {} ({:?})", name, path.display(), strategy);
                    }
                    MirrorAction::NoOp => {
                        println!("ok       {}", name);
                    }
                }
            }
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
    let drift = check(&cfg.install, project, &skills)?;
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
