//! Implementation of `quay profile <action>`.

use crate::args::ProfileAction;
use crate::config_io::{read_user_file, write_user_file};
use quay_core::QuayError;
use serde_json::json;
use std::path::Path;

/// Dispatch a `profile` subcommand.
pub fn run(
    action: ProfileAction,
    project: &Path,
    user_config: Option<&Path>,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    match action {
        ProfileAction::List => list(user_config, json),
        ProfileAction::Current => current(project, user_config, json),
        ProfileAction::Add {
            name,
            email,
            remote,
            activate,
        } => add(
            &name,
            email.as_deref(),
            remote.as_deref(),
            activate,
            user_config,
            json,
        ),
        ProfileAction::Use { name } => use_profile(&name, user_config, json),
        ProfileAction::Remove { name } => remove(&name, user_config, json),
        ProfileAction::Show { name } => show(name.as_deref(), user_config, json),
        ProfileAction::Rename { old, new } => rename(&old, &new, user_config, json),
    }
}

fn list(user_config: Option<&Path>, json: bool) -> Result<(), Box<dyn std::error::Error>> {
    let file = read_user_file(user_config)?;
    if json {
        let entries: Vec<_> = file
            .profiles
            .iter()
            .map(|(name, p)| {
                json!({
                    "name": name,
                    "active": file.active_profile.as_deref() == Some(name.as_str()),
                    "email": p.user.email,
                    "remotes": p.remotes.len(),
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&entries)?);
    } else if file.profiles.is_empty() {
        println!("(no profiles configured — run `quay profile add <name>`)");
    } else {
        for (name, p) in &file.profiles {
            let mark = if file.active_profile.as_deref() == Some(name.as_str()) {
                "* "
            } else {
                "  "
            };
            let email = p.user.email.as_deref().unwrap_or("(no email)");
            println!("{}{}\t{}\t{} remotes", mark, name, email, p.remotes.len());
        }
    }
    Ok(())
}

fn current(
    project: &Path,
    user_config: Option<&Path>,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let project_config = project.join(".quay/config.toml");
    let cfg = quay_core::Config::load_resolved(user_config, Some(&project_config), None)?;
    let file = read_user_file(user_config)?;
    let project_pin = if project_config.exists() {
        let txt = std::fs::read_to_string(&project_config)?;
        toml::from_str::<quay_core::ProjectConfigFile>(&txt)
            .ok()
            .and_then(|f| f.profile)
    } else {
        None
    };
    let name = std::env::var("QUAY_PROFILE")
        .ok()
        .filter(|s| !s.is_empty())
        .or(project_pin)
        .or(file.active_profile.clone())
        .or_else(|| {
            if file.profiles.len() == 1 {
                // SAFETY: len == 1 guarantees next() returns Some.
                Some(file.profiles.keys().next().unwrap().clone())
            } else {
                None
            }
        })
        .unwrap_or_else(|| "(none)".into());
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "profile": name,
                "email": cfg.user.email,
                "remotes": cfg.remotes.len(),
            }))?
        );
    } else {
        println!("{}", name);
    }
    Ok(())
}

fn add(
    name: &str,
    email: Option<&str>,
    remote: Option<&str>,
    activate: bool,
    user_config: Option<&Path>,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(format!(
            "profile name '{}' must be non-empty alphanumeric (with optional - or _)",
            name
        )
        .into());
    }
    let path = user_config.ok_or("--user-config (or HOME) required to add a profile")?;
    let mut file = read_user_file(Some(path))?;
    if file.profiles.contains_key(name) {
        return Err(format!("profile '{}' already exists", name).into());
    }
    let mut profile = quay_core::ProfileFile::default();
    if let Some(e) = email {
        profile.user.email = Some(e.into());
    }
    if let Some(spec) = remote {
        let (rname, rurl) = match spec.split_once('=') {
            Some((n, u)) if !n.is_empty() && !u.is_empty() => (n.to_string(), u.to_string()),
            _ => return Err(format!("--remote must be `<name>=<url>`, got '{}'", spec).into()),
        };
        profile.remotes.insert(
            rname,
            quay_core::RemoteConfig {
                url: rurl,
                default: true,
                provider: None,
                push_mode: quay_core::PushMode::default(),
            },
        );
    }
    file.profiles.insert(name.into(), profile);
    if activate || file.active_profile.is_none() {
        file.active_profile = Some(name.into());
    }
    write_user_file(path, &file)?;

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "profile": name,
                "active": file.active_profile.as_deref() == Some(name),
                "path": path.display().to_string(),
            }))?
        );
    } else {
        println!("added profile '{}'", name);
        if file.active_profile.as_deref() == Some(name) {
            println!("  (also set as active)");
        }
    }
    Ok(())
}

fn use_profile(
    name: &str,
    user_config: Option<&Path>,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let path = user_config.ok_or("--user-config (or HOME) required")?;
    let mut file = read_user_file(Some(path))?;
    if !file.profiles.contains_key(name) {
        return Err(QuayError::ProfileUnknown(name.into()).into());
    }
    file.active_profile = Some(name.into());
    write_user_file(path, &file)?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({"active_profile": name}))?
        );
    } else {
        println!("active profile: {}", name);
    }
    Ok(())
}

fn remove(
    name: &str,
    user_config: Option<&Path>,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let path = user_config.ok_or("--user-config (or HOME) required")?;
    let mut file = read_user_file(Some(path))?;
    if !file.profiles.contains_key(name) {
        return Err(QuayError::ProfileUnknown(name.into()).into());
    }
    if file.profiles.len() == 1 {
        return Err("cannot remove the only profile".into());
    }
    file.profiles.remove(name);
    if file.active_profile.as_deref() == Some(name) {
        file.active_profile = file.profiles.keys().next().cloned();
    }
    write_user_file(path, &file)?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(
                &json!({"removed": name, "active_profile": file.active_profile})
            )?
        );
    } else {
        println!("removed profile '{}'", name);
        if let Some(active) = &file.active_profile {
            println!("  active profile is now: {}", active);
        }
    }
    Ok(())
}

fn show(
    name: Option<&str>,
    user_config: Option<&Path>,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let file = read_user_file(user_config)?;
    let target = match name {
        Some(n) => n.to_string(),
        None => file
            .active_profile
            .clone()
            .ok_or("no active profile and no name passed")?,
    };
    let p = file
        .profiles
        .get(&target)
        .ok_or_else(|| QuayError::ProfileUnknown(target.clone()))?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "name": target,
                "user": p.user,
                "remotes": p.remotes,
            }))?
        );
    } else {
        println!("profile: {}", target);
        println!("  email: {}", p.user.email.as_deref().unwrap_or("(none)"));
        if p.remotes.is_empty() {
            println!("  remotes: (none)");
        } else {
            println!("  remotes:");
            for (rname, r) in &p.remotes {
                let star = if r.default { "*" } else { " " };
                println!("    {} {}\t{}", star, rname, r.url);
            }
        }
    }
    Ok(())
}

fn rename(
    old: &str,
    new: &str,
    user_config: Option<&Path>,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let path = user_config.ok_or("--user-config (or HOME) required")?;
    let mut file = read_user_file(Some(path))?;
    if !file.profiles.contains_key(old) {
        return Err(QuayError::ProfileUnknown(old.into()).into());
    }
    if file.profiles.contains_key(new) {
        return Err(format!("profile '{}' already exists", new).into());
    }
    let p = file.profiles.remove(old).unwrap();
    file.profiles.insert(new.into(), p);
    if file.active_profile.as_deref() == Some(old) {
        file.active_profile = Some(new.into());
    }
    write_user_file(path, &file)?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({"old": old, "new": new}))?
        );
    } else {
        println!("renamed profile '{}' \u{2192} '{}'", old, new);
    }
    Ok(())
}
