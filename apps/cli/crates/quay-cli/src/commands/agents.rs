//! Implementation of `quay agents` — registry-driven mirroring into coding
//! agents' skill directories. Translates agent ids into mirror configs via the
//! built-in registry, then reuses the existing [`quay_core::apply_all`] engine.

use crate::args::AgentsAction;
use crate::config_io::{read_project_file, write_project_file};
use quay_core::{
    agent_registry, apply_all, detect_installed, install_config, AgentScope, InstallConfig,
    MirrorAction,
};
use serde_json::json;
use std::path::Path;

pub fn run(
    action: AgentsAction,
    project: &Path,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    match action {
        AgentsAction::List => list(json),
        AgentsAction::Link {
            agents,
            global,
            force,
        } => link(agents, global, force, project, json),
    }
}

/// Platform-correct home dir (Windows uses the proper API via `dirs`).
fn home_dir() -> Result<String, Box<dyn std::error::Error>> {
    dirs::home_dir()
        .and_then(|p| p.to_str().map(String::from))
        .ok_or_else(|| "cannot determine home directory".into())
}

fn list(json: bool) -> Result<(), Box<dyn std::error::Error>> {
    let reg = agent_registry();
    let home = home_dir()?;
    let installed: std::collections::BTreeSet<String> =
        detect_installed(&reg, &home).into_iter().collect();

    if json {
        let rows: Vec<_> = reg
            .agents
            .iter()
            .map(|(id, a)| {
                json!({
                    "id": id,
                    "display_name": a.display_name,
                    "detected": installed.contains(id),
                    "global": a.global.is_some(),
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&rows)?);
    } else {
        for (id, a) in &reg.agents {
            let mark = if installed.contains(id) { "●" } else { " " };
            println!("{mark} {id:<18} {}", a.display_name);
        }
        println!("\n● = detected on this machine");
    }
    Ok(())
}

fn link(
    agents: Vec<String>,
    global: bool,
    force: bool,
    project: &Path,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let reg = agent_registry();
    let home = home_dir()?;

    // No `-a`: target everything we detect on this machine.
    let targets = if agents.is_empty() {
        let detected = detect_installed(&reg, &home);
        if detected.is_empty() {
            return Err("no agents detected — pass --agent <id> (see `quay agents list`)".into());
        }
        detected
    } else {
        agents
    };

    let scope = if global {
        AgentScope::Global
    } else {
        AgentScope::Project
    };
    let install = install_config(&reg, &targets, scope, &home)?;

    // Global paths are absolute → join with "/" is a no-op; project paths are
    // repo-relative → resolved against the project root. Both handled by apply_all.
    let root = if global { Path::new("/") } else { project };
    let skills = installed_skills(&install, root);
    if skills.is_empty() {
        return Err(format!(
            "no skills found in {} to mirror",
            root.join(&install.canonical).display()
        )
        .into());
    }

    let mut out: Vec<(String, Vec<MirrorAction>)> = Vec::new();
    for name in &skills {
        out.push((name.clone(), apply_all(&install, root, name, force)?));
    }

    // Project scope: record the mirrors in `.quay/config.toml` so `quay link`
    // / `quay link check` tracks them too. Global paths are machine-specific,
    // so they are never persisted. Only touch an already-initialized project.
    if !global && project.join(".quay/config.toml").exists() {
        persist_project_mirrors(&install, project)?;
    }

    if json {
        let payload: Vec<_> = out
            .iter()
            .map(|(s, acts)| json!({ "skill": s, "mirrors": acts.len() }))
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "agents": targets,
                "scope": if global { "global" } else { "project" },
                "results": payload,
            }))?
        );
    } else if install.mirrors.is_empty() {
        println!("(selected agents all read the canonical directly — nothing to mirror)");
    } else {
        for (name, acts) in &out {
            for a in acts {
                match a {
                    MirrorAction::Created { path, .. } => {
                        println!("created  {name} -> {}", path.display())
                    }
                    MirrorAction::Replaced { path, .. } => {
                        println!("replaced {name} -> {}", path.display())
                    }
                    MirrorAction::NoOp => println!("ok       {name}"),
                }
            }
        }
    }
    Ok(())
}

/// Merge freshly-built mirrors into the project config, deduped by path.
fn persist_project_mirrors(
    install: &InstallConfig,
    project: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut file = read_project_file(project)?;
    let mut changed = false;
    for mirror in &install.mirrors {
        if !file.install.mirrors.iter().any(|m| m.path == mirror.path) {
            file.install.mirrors.push(mirror.clone());
            changed = true;
        }
    }
    if changed {
        write_project_file(project, &file)?;
    }
    Ok(())
}

/// Skill names living under the canonical dir (each a subdirectory).
fn installed_skills(install: &InstallConfig, root: &Path) -> Vec<String> {
    let canonical = root.join(&install.canonical);
    let mut names = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&canonical) {
        for e in entries.flatten() {
            if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                if let Some(n) = e.file_name().to_str() {
                    names.push(n.to_string());
                }
            }
        }
    }
    names.sort();
    names
}
