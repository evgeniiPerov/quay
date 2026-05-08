use crate::args::RemoteAction;
use quay_core::{Config, QuayError, RemoteConfig};
use serde_json::json;
use std::path::Path;

fn project_config_path(project: &Path) -> std::path::PathBuf {
    project.join(".quay/config.toml")
}

fn load_project(project: &Path) -> Result<Config, Box<dyn std::error::Error>> {
    Ok(Config::read(&project_config_path(project))?)
}

pub fn run(
    action: RemoteAction,
    project: &Path,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    match action {
        RemoteAction::Add { name, url, default } => {
            let mut cfg = load_project(project)?;
            if cfg.remotes.contains_key(&name) {
                return Err(QuayError::RemoteExists(name).into());
            }
            if default {
                for r in cfg.remotes.values_mut() {
                    r.default = false;
                }
            }
            cfg.remotes.insert(
                name.clone(),
                RemoteConfig {
                    url,
                    default,
                    provider: None,
                },
            );
            cfg.write(&project_config_path(project))?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({"action": "added", "name": name}))?
                );
            } else {
                println!("added remote {}", name);
            }
        }
        RemoteAction::List => {
            let cfg = load_project(project)?;
            if json {
                let arr: Vec<_> = cfg
                    .remotes
                    .iter()
                    .map(|(name, r)| json!({"name": name, "url": r.url, "default": r.default}))
                    .collect();
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({"remotes": arr}))?
                );
            } else if cfg.remotes.is_empty() {
                println!("(no remotes configured)");
            } else {
                for (name, r) in &cfg.remotes {
                    let tag = if r.default { " [default]" } else { "" };
                    println!("{:<24} {}{}", name, r.url, tag);
                }
            }
        }
        RemoteAction::Remove { name } => {
            let mut cfg = load_project(project)?;
            if cfg.remotes.remove(&name).is_none() {
                return Err(QuayError::RemoteUnknown(name).into());
            }
            cfg.write(&project_config_path(project))?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({"action": "removed", "name": name}))?
                );
            } else {
                println!("removed remote {}", name);
            }
        }
    }
    Ok(())
}
