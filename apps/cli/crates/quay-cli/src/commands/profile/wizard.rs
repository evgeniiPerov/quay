//! Interactive wizard for `quay profile add -i`.
//!
//! Chains `dialoguer::Input`, `Select`, and `Confirm` prompts to walk the user
//! through profile creation: name → email → remote(s) loop → activation.
//!
//! This module is CLI-only; the TUI Onboarding screen has its own ratatui-form
//! flow but shares the [`ProfileDraft`] data model and persistence path.

use crate::commands::interactive::is_tty;
use quay_core::{detect_kind_from_url, ProfileDraft, ProviderKind, PushMode, RemoteDraft};

/// Run the interactive wizard and return a completed [`ProfileDraft`].
///
/// Returns an error when stdin is not a TTY or when the user cancels.
pub fn run_wizard() -> Result<ProfileDraft, Box<dyn std::error::Error>> {
    run_wizard_inner(None, None, Vec::new())
}

/// Run the interactive wizard pre-populated with existing values.
///
/// Used by `quay profile edit <name> -i`: the name field is pre-filled and
/// locked to the existing name; email shows current value as default;
/// each existing remote is walked first with keep/edit/remove choices.
///
/// Returns an error when stdin is not a TTY or when the user cancels.
pub fn run_wizard_with_defaults(
    existing_name: &str,
    existing_email: Option<&str>,
    existing_remotes: Vec<RemoteDraft>,
) -> Result<ProfileDraft, Box<dyn std::error::Error>> {
    run_wizard_inner(Some(existing_name), existing_email, existing_remotes)
}

fn run_wizard_inner(
    locked_name: Option<&str>,
    default_email: Option<&str>,
    existing_remotes: Vec<RemoteDraft>,
) -> Result<ProfileDraft, Box<dyn std::error::Error>> {
    if !is_tty() {
        return Err(
            "interactive mode (-i) requires a TTY; stdin is not a terminal. \
             Provide profile details via flags or --from-toml instead."
                .into(),
        );
    }

    // Step 1 — profile name (locked when editing).
    let name = if let Some(n) = locked_name {
        println!("Editing profile '{}'", n);
        n.to_string()
    } else {
        dialoguer::Input::<String>::new()
            .with_prompt("Profile name")
            .validate_with(|s: &String| validate_profile_name(s))
            .interact_text()?
    };

    // Step 2 — email (pre-populated when editing).
    let email_prompt = dialoguer::Input::<String>::new()
        .with_prompt("Author email")
        .validate_with(|s: &String| validate_email_loose(s));
    let email = if let Some(e) = default_email {
        email_prompt.with_initial_text(e).interact_text()?
    } else {
        email_prompt.interact_text()?
    };

    // Step 3 — walk existing remotes first (edit mode), then add-new loop.
    let mut remotes: Vec<RemoteDraft> = Vec::new();
    for existing in &existing_remotes {
        if let Some(updated) = prompt_existing_remote(existing)? {
            remotes.push(updated);
        }
    }
    loop {
        let prompt = if remotes.is_empty() {
            "Add a remote now?"
        } else {
            "Add another remote?"
        };
        let add = dialoguer::Confirm::new()
            .with_prompt(prompt)
            .default(remotes.is_empty())
            .interact()?;
        if !add {
            break;
        }
        remotes.push(prompt_remote(&remotes)?);
    }

    // Step 4 — activation.
    let activate = dialoguer::Confirm::new()
        .with_prompt("Set this profile as active now?")
        .default(true)
        .interact()?;

    // Step 5 — final confirm.
    let save = dialoguer::Confirm::new()
        .with_prompt(format!("Save profile '{}'?", name))
        .default(true)
        .interact()?;
    if !save {
        return Err("wizard cancelled by user".into());
    }

    Ok(ProfileDraft {
        name,
        email,
        remotes,
        activate,
    })
}

/// Prompt for a single remote's details.
fn prompt_remote(existing: &[RemoteDraft]) -> Result<RemoteDraft, Box<dyn std::error::Error>> {
    // Remote name.
    let taken: Vec<&str> = existing.iter().map(|r| r.name.as_str()).collect();
    let rname = dialoguer::Input::<String>::new()
        .with_prompt("Remote name")
        .validate_with(|s: &String| {
            if s.is_empty() {
                return Err("remote name must not be empty".to_string());
            }
            if taken.contains(&s.as_str()) {
                return Err(format!("remote '{}' already added", s));
            }
            Ok(())
        })
        .interact_text()?;

    // URL.
    let rurl = dialoguer::Input::<String>::new()
        .with_prompt("Git URL")
        .validate_with(|s: &String| {
            if s.is_empty() {
                Err("URL must not be empty".to_string())
            } else {
                Ok(())
            }
        })
        .interact_text()?;

    // Provider — auto-detect and let user confirm / override.
    let detected = detect_kind_from_url(&rurl);
    let provider_choices = [
        ProviderKind::GitHub,
        ProviderKind::GitHubEnterprise,
        ProviderKind::GitLab,
        ProviderKind::Bitbucket,
        ProviderKind::AzureDevOps,
    ];
    let provider_labels = [
        "github",
        "github-enterprise",
        "gitlab",
        "bitbucket",
        "azuredevops",
    ];
    let default_provider_idx = provider_choices
        .iter()
        .position(|&p| p == detected)
        .unwrap_or(0);
    let provider_idx = dialoguer::Select::new()
        .with_prompt(format!(
            "Provider (auto-detected: {})",
            provider_labels[default_provider_idx]
        ))
        .items(provider_labels)
        .default(default_provider_idx)
        .interact()?;
    let provider = provider_choices[provider_idx];

    let (push_mode, direct_branch) = prompt_push_mode_and_branch(PushMode::Direct, None)?;

    // Default?
    let is_default = existing.is_empty()
        || dialoguer::Confirm::new()
            .with_prompt("Mark as default remote?")
            .default(false)
            .interact()?;

    Ok(RemoteDraft {
        name: rname,
        url: rurl,
        provider,
        push_mode,
        direct_branch,
        default: is_default,
    })
}

/// Prompt for push_mode + direct_branch with the given defaults preselected.
///
/// Used both for new remotes (in `prompt_remote`) and for editing an existing
/// remote's push behavior without touching its URL/name/provider.
fn prompt_push_mode_and_branch(
    default_mode: PushMode,
    default_branch: Option<&str>,
) -> Result<(PushMode, Option<String>), Box<dyn std::error::Error>> {
    let mode_labels = ["pr (open a pull request)", "direct (push to target branch)"];
    let default_idx = match default_mode {
        PushMode::Pr => 0,
        PushMode::Direct => 1,
    };
    let mode_idx = dialoguer::Select::new()
        .with_prompt("Push mode")
        .items(mode_labels)
        .default(default_idx)
        .interact()?;
    let push_mode = if mode_idx == 0 {
        PushMode::Pr
    } else {
        PushMode::Direct
    };

    let direct_branch = if matches!(push_mode, PushMode::Direct) {
        let mut input = dialoguer::Input::<String>::new()
            .with_prompt("Direct push branch (empty = hub default)")
            .allow_empty(true);
        if let Some(b) = default_branch {
            input = input.with_initial_text(b);
        }
        let s = input.interact_text()?;
        let trimmed = s.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    } else {
        None
    };
    Ok((push_mode, direct_branch))
}

/// Walk an existing remote in edit mode. Returns:
/// * `Ok(Some(updated))` — keep (optionally with edited push behavior)
/// * `Ok(None)` — user chose to remove this remote
fn prompt_existing_remote(
    rd: &RemoteDraft,
) -> Result<Option<RemoteDraft>, Box<dyn std::error::Error>> {
    let summary = format!(
        "Remote '{}' [{}] (provider={:?}, push_mode={:?}{}{}{}",
        rd.name,
        rd.url,
        rd.provider,
        rd.push_mode,
        rd.direct_branch
            .as_deref()
            .map(|b| format!(", direct_branch={}", b))
            .unwrap_or_default(),
        if rd.default { ", default" } else { "" },
        ")"
    );
    println!("{}", summary);

    let actions = [
        "keep as-is",
        "change push_mode / direct_branch only",
        "remove this remote",
    ];
    let choice = dialoguer::Select::new()
        .with_prompt("What to do with this remote?")
        .items(actions)
        .default(0)
        .interact()?;

    match choice {
        0 => Ok(Some(rd.clone())),
        1 => {
            let (push_mode, direct_branch) =
                prompt_push_mode_and_branch(rd.push_mode, rd.direct_branch.as_deref())?;
            Ok(Some(RemoteDraft {
                push_mode,
                direct_branch,
                ..rd.clone()
            }))
        }
        _ => Ok(None),
    }
}

// ---------------------------------------------------------------------------
// Dialoguer adapters — convert quay_core::validate errors into the `String`
// shape dialoguer's `.validate_with` callback expects.
// ---------------------------------------------------------------------------

/// Thin adapter over [`quay_core::validate::profile_name`] for dialoguer.
pub fn validate_profile_name(name: &str) -> Result<(), String> {
    quay_core::validate::profile_name(name).map_err(|e| e.to_string())
}

/// Thin adapter over [`quay_core::validate::email_loose`] for dialoguer.
pub fn validate_email_loose(email: &str) -> Result<(), String> {
    quay_core::validate::email_loose(email).map_err(|e| e.to_string())
}
