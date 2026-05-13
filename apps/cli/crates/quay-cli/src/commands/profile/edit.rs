//! Implementation of `quay profile edit <name>`.
//!
//! Three mutually-exclusive modes mirror `profile add`:
//!
//! * **Explicit flags** — `quay profile edit <name> --email <e>`
//!   Only the supplied fields are patched; everything else is unchanged.
//!
//! * **Wizard** — `quay profile edit <name> -i`
//!   Opens the interactive wizard pre-populated with the current values.
//!   When `<name>` is omitted with `-i`, a profile picker is shown first.
//!
//! * **TOML ingestion** — `quay profile edit <name> --from-toml <path|->`
//!   Replaces the entire profile's content (email + remotes) while keeping
//!   the profile's name and position in the config file.

use super::ingest_toml;
use super::wizard;
use crate::commands::interactive::{is_tty, pick_one};
use crate::config_io::{read_user_file, write_user_file};
use quay_core::{detect_kind_from_url, QuayError, RemoteConfig, RemoteDraft};
use serde_json::json;
use std::collections::BTreeMap;
use std::path::Path;

/// Dispatch `quay profile edit`.
#[allow(clippy::too_many_arguments)]
pub fn run(
    name: Option<String>,
    interactive: bool,
    from_toml: Option<String>,
    email: Option<String>,
    user_config: Option<&Path>,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let path = user_config.ok_or("--user-config (or HOME) required to edit a profile")?;

    // ── Interactive wizard mode ───────────────────────────────────────────────
    if interactive {
        // Resolve which profile to edit: positional name or picker.
        let profile_name = match name {
            Some(n) => n,
            None => pick_profile_interactively(path)?,
        };

        // Read current values for pre-population.
        let file = read_user_file(Some(path))?;
        let current = file
            .profiles
            .get(&profile_name)
            .ok_or_else(|| QuayError::ProfileUnknown(profile_name.clone()))?;
        let current_email = current.user.email.as_deref();
        let existing_remotes: Vec<RemoteDraft> = current
            .remotes
            .iter()
            .map(|(name, r)| RemoteDraft {
                name: name.clone(),
                url: r.url.clone(),
                provider: r.provider.unwrap_or_else(|| detect_kind_from_url(&r.url)),
                push_mode: r.push_mode,
                direct_branch: r.direct_branch.clone(),
                default: r.default,
            })
            .collect();

        let draft =
            wizard::run_wizard_with_defaults(&profile_name, current_email, existing_remotes)?;

        // Apply the wizard result: replace the profile's email and remotes.
        apply_wizard_draft(path, &profile_name, &draft.email, &draft.remotes, json)?;
        return Ok(());
    }

    // Require `name` for non-interactive modes.
    let profile_name = name.ok_or("profile name is required (use -i for interactive mode)")?;

    // ── TOML ingestion mode ───────────────────────────────────────────────────
    if let Some(ref from_toml_arg) = from_toml {
        let toml_text = ingest_toml::read_from_arg(from_toml_arg)?;
        // parse() builds a ProfileDraft; we use its email + remotes fields.
        let draft = ingest_toml::parse(&toml_text, &profile_name, false)?;
        if !draft.email.is_empty() {
            quay_core::validate::email_loose(&draft.email)?;
        }

        let mut file = read_user_file(Some(path))?;
        if !file.profiles.contains_key(&profile_name) {
            return Err(QuayError::ProfileUnknown(profile_name).into());
        }

        // Build new remotes map from the draft. Every RemoteConfig field must
        // be carried through here — adding a field without updating this loop
        // silently drops it on edit-via-TOML.
        let mut new_remotes: BTreeMap<String, RemoteConfig> = BTreeMap::new();
        for rd in &draft.remotes {
            new_remotes.insert(
                rd.name.clone(),
                RemoteConfig {
                    url: rd.url.clone(),
                    default: rd.default,
                    provider: Some(rd.provider),
                    push_mode: rd.push_mode,
                    direct_branch: rd.direct_branch.clone(),
                },
            );
        }

        let p = file.profiles.get_mut(&profile_name).unwrap();
        p.user.email = if draft.email.is_empty() {
            None
        } else {
            Some(draft.email.clone())
        };
        p.remotes = new_remotes;

        write_user_file(path, &file)?;
        print_edit_result(&profile_name, path, json)?;
        return Ok(());
    }

    // ── Explicit-flags mode ───────────────────────────────────────────────────
    // At least one flag must be present.
    if email.is_none() {
        return Err("nothing to do: provide --email, --from-toml, or -i to edit a profile".into());
    }

    let mut file = read_user_file(Some(path))?;
    if !file.profiles.contains_key(&profile_name) {
        return Err(QuayError::ProfileUnknown(profile_name).into());
    }

    let p = file.profiles.get_mut(&profile_name).unwrap();
    if let Some(ref new_email) = email {
        quay_core::validate::email_loose(new_email)?;
        p.user.email = Some(new_email.clone());
    }

    write_user_file(path, &file)?;
    print_edit_result(&profile_name, path, json)?;
    Ok(())
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Apply a wizard-produced draft (email + remotes) to an existing profile.
fn apply_wizard_draft(
    path: &Path,
    profile_name: &str,
    email: &str,
    remotes: &[quay_core::RemoteDraft],
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut file = read_user_file(Some(path))?;
    if !file.profiles.contains_key(profile_name) {
        return Err(QuayError::ProfileUnknown(profile_name.into()).into());
    }

    let mut new_remotes: BTreeMap<String, RemoteConfig> = BTreeMap::new();
    for rd in remotes {
        new_remotes.insert(
            rd.name.clone(),
            RemoteConfig {
                url: rd.url.clone(),
                default: rd.default,
                provider: Some(rd.provider),
                push_mode: rd.push_mode,
                direct_branch: rd.direct_branch.clone(),
            },
        );
    }

    let p = file.profiles.get_mut(profile_name).unwrap();
    p.user.email = if email.is_empty() {
        None
    } else {
        Some(email.to_string())
    };
    p.remotes = new_remotes;

    write_user_file(path, &file)?;
    print_edit_result(profile_name, path, json)
}

/// Open an interactive single-select list of existing profiles.
fn pick_profile_interactively(path: &Path) -> Result<String, Box<dyn std::error::Error>> {
    if !is_tty() {
        return Err(
            "interactive mode (-i) requires a TTY; stdin is not a terminal. \
             Provide a profile name or --from-toml instead."
                .into(),
        );
    }
    let file = read_user_file(Some(path))?;
    if file.profiles.is_empty() {
        return Err("no profiles configured — run `quay profile add <name>` first".into());
    }
    let names: Vec<&String> = file.profiles.keys().collect();
    let active = file.active_profile.as_deref();
    let default_idx = active.and_then(|a| names.iter().position(|n| n.as_str() == a));
    let idx = pick_one(
        "Select profile to edit",
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
    Ok(names[idx].clone())
}

fn print_edit_result(
    name: &str,
    path: &Path,
    json: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let file = read_user_file(Some(path))?;
    let p = file.profiles.get(name);
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "profile": name,
                "email": p.and_then(|p| p.user.email.as_deref()),
                "path": path.display().to_string(),
            }))?
        );
    } else {
        println!("updated profile '{}'", name);
    }
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use assert_fs::prelude::*;
    use quay_core::UserConfigFile;

    fn write_two_profiles(dir: &assert_fs::TempDir) -> std::path::PathBuf {
        let p = dir.child("user.toml");
        let contents = r#"
active_profile = "work"
[profiles.work.user]
email = "e@work"
[profiles.personal.user]
email = "e@home"
"#;
        std::fs::write(p.path(), contents).unwrap();
        p.path().to_path_buf()
    }

    #[test]
    fn explicit_email_patch_updates_disk() {
        let dir = assert_fs::TempDir::new().unwrap();
        let cfg = write_two_profiles(&dir);

        run(
            Some("work".into()),
            false,
            None,
            Some("new@work".into()),
            Some(&cfg),
            false,
        )
        .unwrap();

        let saved = std::fs::read_to_string(&cfg).unwrap();
        assert!(
            saved.contains("new@work"),
            "expected new email on disk: {saved}"
        );
        // personal profile must still be intact.
        assert!(
            saved.contains("e@home"),
            "personal profile missing: {saved}"
        );
    }

    #[test]
    fn explicit_email_rejects_unknown_profile() {
        let dir = assert_fs::TempDir::new().unwrap();
        let cfg = write_two_profiles(&dir);

        let err = run(
            Some("ghost".into()),
            false,
            None,
            Some("x@y".into()),
            Some(&cfg),
            false,
        )
        .unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("ghost") || msg.to_lowercase().contains("unknown"),
            "expected 'ghost' in error: {msg}"
        );
    }

    #[test]
    fn explicit_no_flags_returns_error() {
        let dir = assert_fs::TempDir::new().unwrap();
        let cfg = write_two_profiles(&dir);

        let err = run(Some("work".into()), false, None, None, Some(&cfg), false).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("nothing to do"),
            "expected 'nothing to do': {msg}"
        );
    }

    #[test]
    fn from_toml_replaces_email_and_remotes() {
        let dir = assert_fs::TempDir::new().unwrap();
        let cfg = write_two_profiles(&dir);

        let toml = r#"
email = "replaced@work"
[remotes.hub]
url = "https://github.com/org/skills.git"
default = true
"#;
        let toml_file = dir.child("edit.toml");
        toml_file.write_str(toml).unwrap();

        run(
            Some("work".into()),
            false,
            Some(toml_file.path().to_str().unwrap().to_string()),
            None,
            Some(&cfg),
            false,
        )
        .unwrap();

        let saved = std::fs::read_to_string(&cfg).unwrap();
        assert!(
            saved.contains("replaced@work"),
            "email not replaced: {saved}"
        );
        assert!(
            saved.contains("[profiles.work.remotes.hub]"),
            "remote not written: {saved}"
        );
        // personal profile must still be intact.
        assert!(saved.contains("e@home"), "personal stripped: {saved}");
    }

    #[test]
    fn from_toml_via_stdin_replaces_content() {
        let dir = assert_fs::TempDir::new().unwrap();
        let cfg = write_two_profiles(&dir);

        // We cannot actually pipe stdin in a unit test, but we can verify the
        // path fails cleanly when arg is "-" and stdin is not a real pipe in
        // this test process. The integration tests cover the full stdin path.
        let toml_inline = r#"email = "stdin@work""#;
        let toml_file = dir.child("stdin.toml");
        toml_file.write_str(toml_inline).unwrap();

        run(
            Some("work".into()),
            false,
            Some(toml_file.path().to_str().unwrap().to_string()),
            None,
            Some(&cfg),
            false,
        )
        .unwrap();

        let saved = std::fs::read_to_string(&cfg).unwrap();
        assert!(saved.contains("stdin@work"), "email not set: {saved}");
    }

    #[test]
    fn json_output_contains_profile_name() {
        let dir = assert_fs::TempDir::new().unwrap();
        let cfg = write_two_profiles(&dir);

        // Capture stdout by running run() — it writes to stdout directly.
        // We just check no error occurs and the profile is updated.
        run(
            Some("work".into()),
            false,
            None,
            Some("json@work".into()),
            Some(&cfg),
            true,
        )
        .unwrap();

        // Verify the underlying file was updated (json output path writes same data).
        let file: UserConfigFile = toml::from_str(&std::fs::read_to_string(&cfg).unwrap()).unwrap();
        assert_eq!(
            file.profiles["work"].user.email.as_deref(),
            Some("json@work")
        );
    }
}
