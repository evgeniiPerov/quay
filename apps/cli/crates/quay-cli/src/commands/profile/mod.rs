//! Implementation of `quay profile <action>`.

pub mod ingest_toml;
pub mod wizard;

use crate::args::{ProfileAction, ProviderKindArg, PushModeArg};
use crate::commands::interactive::pick_one;
use crate::config_io::{read_user_file, write_user_file};
use quay_core::{
    detect_kind_from_url, ProfileDraft, ProviderKind, PushMode, QuayError, RemoteDraft,
};
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
            interactive,
            from_toml,
            email,
            remote,
            provider,
            push_mode,
            default,
            activate,
        } => add(
            name,
            interactive,
            from_toml,
            email,
            remote,
            provider,
            push_mode,
            default,
            activate,
            user_config,
            json,
        ),
        ProfileAction::Use { name, interactive } => {
            if interactive {
                use_profile_interactive(user_config, json)
            } else {
                let name = name.ok_or("profile name is required when not using --interactive")?;
                use_profile(&name, user_config, json)
            }
        }
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

/// Build a `RemoteDraft` from the `i`-th `--remote name=url` spec, looking up
/// optional parallel `--provider`, `--push-mode`, `--default` by index.
fn build_remote_draft(
    i: usize,
    spec: &str,
    providers: &[ProviderKindArg],
    push_modes: &[PushModeArg],
    default_count: u8,
) -> Result<RemoteDraft, Box<dyn std::error::Error>> {
    let (rname, rurl) = match spec.split_once('=') {
        Some((n, u)) if !n.is_empty() && !u.is_empty() => (n.to_string(), u.to_string()),
        _ => return Err(format!("--remote must be `<name>=<url>`, got '{}'", spec).into()),
    };

    let provider: ProviderKind = providers
        .get(i)
        .copied()
        .map(ProviderKind::from)
        .unwrap_or_else(|| detect_kind_from_url(&rurl));

    let push_mode: PushMode = push_modes
        .get(i)
        .copied()
        .map(PushMode::from)
        .unwrap_or_default();

    // remote[i] is the default if i < default_count
    let is_default = (i as u8) < default_count;

    Ok(RemoteDraft {
        name: rname,
        url: rurl,
        provider,
        push_mode,
        default: is_default,
    })
}

#[allow(clippy::too_many_arguments)]
fn add(
    name: Option<String>,
    interactive: bool,
    from_toml: Option<String>,
    email: Option<String>,
    remotes_raw: Vec<String>,
    providers: Vec<ProviderKindArg>,
    push_modes: Vec<PushModeArg>,
    default_count: u8,
    activate: bool,
    user_config: Option<&Path>,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let path = user_config.ok_or("--user-config (or HOME) required to add a profile")?;

    if interactive {
        // Wizard mode — name comes from the wizard itself.
        let draft = wizard::run_wizard()?;
        let profile_name = draft.name.clone();
        draft.write_to_user_config(path)?;
        print_add_result(&profile_name, activate, path, json)?;
        return Ok(());
    }

    if let Some(ref from_toml_arg) = from_toml {
        // TOML ingestion mode — name comes from the CLI positional.
        let profile_name = name.ok_or(
            "profile name is required when using --from-toml (e.g. `quay profile add <name> --from-toml …`)",
        )?;
        validate_profile_name(&profile_name)?;
        let toml_text = ingest_toml::read_from_arg(from_toml_arg)?;
        let draft = ingest_toml::parse(&toml_text, &profile_name, activate)?;
        draft.write_to_user_config(path)?;
        print_add_result(&profile_name, activate, path, json)?;
        return Ok(());
    }

    // Explicit-flags mode.
    let profile_name = name.ok_or("profile name is required")?;
    validate_profile_name(&profile_name)?;

    // Validate provider/push_mode counts don't exceed remote count.
    if providers.len() > remotes_raw.len() {
        return Err(format!(
            "--provider specified {} times but only {} --remote(s) given",
            providers.len(),
            remotes_raw.len()
        )
        .into());
    }
    if push_modes.len() > remotes_raw.len() {
        return Err(format!(
            "--push-mode specified {} times but only {} --remote(s) given",
            push_modes.len(),
            remotes_raw.len()
        )
        .into());
    }

    // Auto-mark the first remote as default when there is exactly one remote
    // and the user did not pass --default, preserving backward compatibility.
    let effective_default_count = if default_count == 0 && remotes_raw.len() == 1 {
        1
    } else {
        default_count
    };

    let mut remote_drafts: Vec<RemoteDraft> = Vec::new();
    for (i, spec) in remotes_raw.iter().enumerate() {
        remote_drafts.push(build_remote_draft(
            i,
            spec,
            &providers,
            &push_modes,
            effective_default_count,
        )?);
    }

    let draft = ProfileDraft {
        name: profile_name.clone(),
        email: email.unwrap_or_default(),
        remotes: remote_drafts,
        activate,
    };
    draft.write_to_user_config(path)?;
    print_add_result(&profile_name, activate, path, json)?;
    Ok(())
}

/// Validate profile name against `^[a-z0-9][a-z0-9_-]*$` or single char `[a-z0-9]`.
fn validate_profile_name(name: &str) -> Result<(), Box<dyn std::error::Error>> {
    if name.is_empty() {
        return Err("profile name must not be empty".into());
    }
    let first = name.chars().next().unwrap();
    if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
        return Err(format!(
            "profile name '{}' must start with a lowercase letter or digit",
            name
        )
        .into());
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
    {
        return Err(format!(
            "profile name '{}' must only contain lowercase letters, digits, hyphens, or underscores",
            name
        )
        .into());
    }
    if name.len() > 64 {
        return Err(format!("profile name '{}' exceeds 64 characters", name).into());
    }
    Ok(())
}

fn print_add_result(
    name: &str,
    activate: bool,
    path: &Path,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    // Re-read the file to get the current active_profile state.
    let file = read_user_file(Some(path))?;
    let is_active = file.active_profile.as_deref() == Some(name);

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "profile": name,
                "active": is_active,
                "path": path.display().to_string(),
            }))?
        );
    } else {
        println!("added profile '{}'", name);
        if is_active {
            println!("  (also set as active)");
        }
    }
    let _ = activate; // activate is embedded in ProfileDraft; nothing more to do here
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
        println!("active profile \u{2192} {}", name);
    }
    Ok(())
}

fn use_profile_interactive(
    user_config: Option<&Path>,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let path = user_config.ok_or("--user-config (or HOME) required")?;
    let file = read_user_file(Some(path))?;
    if file.profiles.is_empty() {
        return Err("no profiles configured — run `quay profile add <name>`".into());
    }
    let names: Vec<&String> = file.profiles.keys().collect();
    let active = file.active_profile.as_deref();
    let default_idx = active.and_then(|a| names.iter().position(|n| n.as_str() == a));
    let picked_idx = pick_one(
        "Select active profile",
        &names,
        |n| {
            if active == Some(n.as_str()) {
                format!("{} *", n)
            } else {
                n.to_string()
            }
        },
        default_idx,
    )?;
    let picked_name = names[picked_idx].as_str();
    use_profile(picked_name, user_config, json)
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
