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
    if !is_tty() {
        return Err(
            "interactive mode (-i) requires a TTY; stdin is not a terminal. \
             Provide profile details via flags or --from-toml instead."
                .into(),
        );
    }

    // Step 1 — profile name.
    let name = dialoguer::Input::<String>::new()
        .with_prompt("Profile name")
        .validate_with(|s: &String| validate_profile_name(s))
        .interact_text()?;

    // Step 2 — email.
    let email = dialoguer::Input::<String>::new()
        .with_prompt("Author email")
        .validate_with(|s: &String| validate_email_loose(s))
        .interact_text()?;

    // Step 3 — remote loop.
    let mut remotes: Vec<RemoteDraft> = Vec::new();
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
        .items(&provider_labels)
        .default(default_provider_idx)
        .interact()?;
    let provider = provider_choices[provider_idx];

    // Push mode.
    let mode_labels = [
        "pr (open a pull request)",
        "direct (push to default branch)",
    ];
    let mode_idx = dialoguer::Select::new()
        .with_prompt("Push mode")
        .items(&mode_labels)
        .default(0)
        .interact()?;
    let push_mode = if mode_idx == 0 {
        PushMode::Pr
    } else {
        PushMode::Direct
    };

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
        default: is_default,
    })
}

// ---------------------------------------------------------------------------
// Validation helpers (pure — unit-testable without a TTY)
// ---------------------------------------------------------------------------

/// Validate that a profile name matches `^[a-z0-9][a-z0-9_-]*$`, max 64 chars.
pub fn validate_profile_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("profile name must not be empty".into());
    }
    let first = name.chars().next().unwrap();
    if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
        return Err("profile name must start with a lowercase letter or digit".into());
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
    {
        return Err(
            "profile name may only contain lowercase letters, digits, hyphens, underscores".into(),
        );
    }
    if name.len() > 64 {
        return Err(format!(
            "profile name exceeds 64 characters (got {})",
            name.len()
        ));
    }
    Ok(())
}

/// Loose email validation: non-empty, contains `@`, no whitespace.
pub fn validate_email_loose(email: &str) -> Result<(), String> {
    if email.is_empty() {
        return Err("email must not be empty".into());
    }
    if !email.contains('@') {
        return Err("email must contain '@'".into());
    }
    if email.chars().any(|c| c.is_whitespace()) {
        return Err("email must not contain whitespace".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_profile_name_rejects_uppercase() {
        assert!(validate_profile_name("Work").is_err());
        assert!(validate_profile_name("WORK").is_err());
        assert!(validate_profile_name("workSpace").is_err());
    }

    #[test]
    fn validate_profile_name_rejects_leading_special() {
        assert!(validate_profile_name("-work").is_err());
        assert!(validate_profile_name("_work").is_err());
        assert!(validate_profile_name("").is_err());
    }

    #[test]
    fn validate_profile_name_accepts_valid() {
        assert!(validate_profile_name("work").is_ok());
        assert!(validate_profile_name("my-profile").is_ok());
        assert!(validate_profile_name("work_2024").is_ok());
        assert!(validate_profile_name("p").is_ok());
        assert!(validate_profile_name("a1").is_ok());
    }

    #[test]
    fn validate_profile_name_rejects_too_long() {
        let long = "a".repeat(65);
        assert!(validate_profile_name(&long).is_err());
    }

    #[test]
    fn validate_email_loose_requires_at_sign() {
        assert!(validate_email_loose("notanemail").is_err());
        assert!(validate_email_loose("").is_err());
        assert!(validate_email_loose("a @b.com").is_err()); // whitespace
        assert!(validate_email_loose("a@b.com").is_ok());
        assert!(validate_email_loose("x@y").is_ok());
    }
}
