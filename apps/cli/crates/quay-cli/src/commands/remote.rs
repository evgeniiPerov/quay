use crate::args::RemoteAction;
use quay_core::{Config, ConnectionStatus, QuayError, RemoteConfig};
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
    user_config: Option<&Path>,
    profile: Option<&str>,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    match action {
        RemoteAction::Add {
            name,
            url,
            default,
            provider,
            push_mode,
            direct_branch,
        } => {
            let mut cfg = load_project(project)?;
            if cfg.remotes.contains_key(&name) {
                return Err(QuayError::RemoteExists(name).into());
            }
            if default {
                for r in cfg.remotes.values_mut() {
                    r.default = false;
                }
            }
            let kind: Option<quay_core::ProviderKind> = provider.map(Into::into);
            // An empty string on the CLI means "no branch override".
            let direct_branch_value = direct_branch.filter(|s| !s.is_empty());
            cfg.remotes.insert(
                name.clone(),
                RemoteConfig {
                    url,
                    default,
                    provider: kind,
                    push_mode: push_mode.map(Into::into).unwrap_or_default(),
                    direct_branch: direct_branch_value,
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
        RemoteAction::Test { name } => {
            let project_config = project_config_path(project);
            let cfg = Config::load_resolved(
                user_config,
                Some(project_config.as_path()).filter(|p| p.exists()),
                profile,
            )?;
            let remote = cfg
                .remotes
                .get(&name)
                .ok_or_else(|| QuayError::ConfigValidation(format!("remote '{}' not found", name)))?
                .clone();
            let provider = quay_core::provider_for_remote(&remote.url, remote.provider);
            let status = provider.test_connection(&remote.url)?;
            if json {
                let (kind, message, size) = match &status {
                    ConnectionStatus::Ok {
                        registry_size_bytes,
                    } => ("ok", String::new(), Some(*registry_size_bytes)),
                    ConnectionStatus::AuthFailed(m) => ("auth_failed", m.clone(), None),
                    ConnectionStatus::Unreachable(m) => ("unreachable", m.clone(), None),
                    ConnectionStatus::NoRegistry(m) => ("no_registry", m.clone(), None),
                };
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({
                        "name": name,
                        "url": remote.url,
                        "status": kind,
                        "message": message,
                        "registry_size_bytes": size,
                    }))?
                );
            } else {
                match &status {
                    ConnectionStatus::Ok {
                        registry_size_bytes,
                    } => {
                        println!(
                            "\u{2713} {} ({}) \u{2014} registry.json {} bytes",
                            name, remote.url, registry_size_bytes
                        );
                    }
                    ConnectionStatus::AuthFailed(msg) => {
                        eprintln!(
                            "\u{2717} {} ({}) \u{2014} auth failed: {}",
                            name, remote.url, msg
                        );
                    }
                    ConnectionStatus::Unreachable(msg) => {
                        eprintln!(
                            "\u{2717} {} ({}) \u{2014} unreachable: {}",
                            name, remote.url, msg
                        );
                    }
                    ConnectionStatus::NoRegistry(msg) => {
                        eprintln!(
                            "\u{2717} {} ({}) \u{2014} no registry: {}",
                            name, remote.url, msg
                        );
                    }
                }
                if !matches!(status, ConnectionStatus::Ok { .. }) {
                    std::process::exit(1);
                }
            }
        }
        RemoteAction::List => {
            // Merge user-level + project-level so users see all remotes
            // available to them (the active profile's remotes from
            // ~/.config/quay/config.toml plus any project overrides).
            let project_config = project_config_path(project);
            let project_path_arg = if project_config.exists() {
                Some(project_config.as_path())
            } else {
                None
            };
            let cfg = Config::load_resolved(user_config, project_path_arg, profile)?;
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
        RemoteAction::Edit {
            name,
            url,
            provider,
            push_mode,
            direct_branch,
            default,
        } => {
            let mut cfg = load_project(project)?;
            let remote = cfg
                .remotes
                .get_mut(&name)
                .ok_or_else(|| QuayError::RemoteUnknown(name.clone()))?;

            // Patch only the supplied fields.
            if let Some(new_url) = url {
                if new_url.is_empty() {
                    return Err("--url must not be empty".into());
                }
                remote.url = new_url;
            }
            if let Some(p) = provider {
                remote.provider = Some(quay_core::ProviderKind::from(p));
            }
            if let Some(pm) = push_mode {
                remote.push_mode = quay_core::PushMode::from(pm);
            }
            if let Some(branch) = direct_branch {
                // Empty string clears the override; any other value sets it.
                remote.direct_branch = if branch.is_empty() {
                    None
                } else {
                    Some(branch)
                };
            }
            if default {
                // Clear the default flag on all other remotes.
                for (n, r) in cfg.remotes.iter_mut() {
                    if n != &name {
                        r.default = false;
                    }
                }
                cfg.remotes.get_mut(&name).unwrap().default = true;
            }

            cfg.write(&project_config_path(project))?;
            if json {
                let r = cfg.remotes.get(&name).unwrap();
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({
                        "action": "edited",
                        "name": name,
                        "url": r.url,
                        "default": r.default,
                        "push_mode": format!("{:?}", r.push_mode).to_lowercase(),
                    }))?
                );
            } else {
                println!("updated remote '{}'", name);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::args::{ProviderKindArg, RemoteAction};
    use assert_fs::TempDir;
    use std::fs;

    fn init_project(dir: &TempDir) {
        // Create a minimal .quay/config.toml so Config::read succeeds.
        let quay_dir = dir.path().join(".quay");
        fs::create_dir_all(&quay_dir).unwrap();
        fs::write(quay_dir.join("config.toml"), "").unwrap();
    }

    #[test]
    fn add_persists_provider_field() {
        let dir = TempDir::new().unwrap();
        init_project(&dir);

        run(
            RemoteAction::Add {
                name: "hub".to_string(),
                url: "https://gitlab.com/org/skills.git".to_string(),
                default: false,
                provider: Some(ProviderKindArg::Gitlab),
                push_mode: None,
                direct_branch: None,
            },
            dir.path(),
            None,
            None,
            false,
        )
        .unwrap();

        let toml_text = fs::read_to_string(dir.path().join(".quay/config.toml")).unwrap();
        assert!(
            toml_text.contains("provider = \"gitlab\""),
            "expected provider field in TOML, got:\n{}",
            toml_text
        );
    }

    #[test]
    fn add_without_provider_omits_field() {
        let dir = TempDir::new().unwrap();
        init_project(&dir);

        run(
            RemoteAction::Add {
                name: "hub".to_string(),
                url: "https://github.com/org/skills.git".to_string(),
                default: false,
                provider: None,
                push_mode: None,
                direct_branch: None,
            },
            dir.path(),
            None,
            None,
            false,
        )
        .unwrap();

        let toml_text = fs::read_to_string(dir.path().join(".quay/config.toml")).unwrap();
        assert!(
            !toml_text.contains("provider"),
            "expected no provider field in TOML, got:\n{}",
            toml_text
        );
    }

    #[test]
    fn add_with_push_mode_direct_persists_field() {
        let dir = TempDir::new().unwrap();
        init_project(&dir);

        run(
            RemoteAction::Add {
                name: "hub".to_string(),
                url: "git@example.com:o/r.git".to_string(),
                default: false,
                provider: None,
                push_mode: Some(crate::args::PushModeArg::Direct),
                direct_branch: None,
            },
            dir.path(),
            None,
            None,
            false,
        )
        .unwrap();

        let toml_text = fs::read_to_string(dir.path().join(".quay/config.toml")).unwrap();
        assert!(
            toml_text.contains("push_mode = \"direct\""),
            "expected push_mode = direct in TOML, got:\n{}",
            toml_text
        );
    }

    #[test]
    fn add_without_push_mode_defaults_to_pr_in_struct() {
        let dir = TempDir::new().unwrap();
        init_project(&dir);

        run(
            RemoteAction::Add {
                name: "hub".to_string(),
                url: "git@example.com:o/r.git".to_string(),
                default: false,
                provider: None,
                push_mode: None,
                direct_branch: None,
            },
            dir.path(),
            None,
            None,
            false,
        )
        .unwrap();

        let cfg = quay_core::Config::read(&dir.path().join(".quay/config.toml")).unwrap();
        assert_eq!(
            cfg.remotes.get("hub").unwrap().push_mode,
            quay_core::PushMode::Pr
        );
    }
}
