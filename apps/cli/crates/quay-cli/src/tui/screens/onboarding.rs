//! First-run onboarding screen — a two-step guided form that runs in place of
//! the dashboard when no user config file exists or `meta.onboarded == false`
//! and no profiles are configured.
//!
//! Step 1: Profile — collect a profile name and identity (email).
//! Step 2: Remote  — collect a hub name and git URL.
//!
//! "Submit" persists a fully-configured user config.
//! `Esc` on Step 1 writes the onboarded marker only (skip path).
//! `Esc` on Step 2 goes back to Step 1.

use crate::config_io::write_user_file;
use crate::tui::app::{App, ScreenAction};
use crate::tui::theme;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use quay_core::{MetaSection, ProfileFile, RemoteConfig, UserConfigFile, UserSection};
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;
use ratatui_form::{Form, FormResult, Pattern};
use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

// ---------------------------------------------------------------------------
// ProfileFields — carries Step 1 values into Step 2
// ---------------------------------------------------------------------------

/// Submitted values from Step 1 carried into Step 2.
#[derive(Debug, Clone)]
pub struct ProfileFields {
    pub name: String,
    pub identity: String,
}

// ---------------------------------------------------------------------------
// State machine
// ---------------------------------------------------------------------------

/// Full onboarding state machine.
pub enum OnboardingState {
    /// Step 1: collect profile name + identity via a `ratatui-form` form.
    Profile { form: Form },
    /// Step 2: collect remote hub name + git URL via a `ratatui-form` form.
    Remote { profile: ProfileFields, form: Form },
    /// A write completed successfully; the app loop will switch to Dashboard.
    Saving,
    /// A write failed.
    Failed(String),
}

impl Default for OnboardingState {
    fn default() -> Self {
        let identity = detect_identity();
        OnboardingState::Profile {
            form: build_profile_form(identity.as_deref()),
        }
    }
}

// ---------------------------------------------------------------------------
// Form builders
// ---------------------------------------------------------------------------

/// Build the Step 1 (Profile) form, optionally pre-filling identity from git.
pub fn build_profile_form(detected_identity: Option<&str>) -> Form {
    Form::builder()
        .title("Step 1 of 2 — Create your first profile")
        .style(crate::tui::form_theme::dark())
        .text("name", "Profile name")
        .placeholder("personal")
        .required()
        .validator(Box::new(Pattern::new(
            r"^[a-z0-9-]+$",
            "lowercase letters, digits, hyphens only",
        )))
        .done()
        .text("identity", "Identity (email)")
        .placeholder("you@example.com")
        .initial_value(detected_identity.unwrap_or(""))
        .required()
        .done()
        .build()
}

/// Build the Step 2 (Remote hub) form.
pub fn build_remote_form() -> Form {
    Form::builder()
        .title("Step 2 of 2 — Add a remote skill hub")
        .style(crate::tui::form_theme::dark())
        .text("hub_name", "Hub name")
        .required()
        .placeholder("skills-hub")
        .done()
        .text("hub_url", "Git URL")
        .required()
        .placeholder("git@github.com:org/skills.git")
        .done()
        .build()
}

// ---------------------------------------------------------------------------
// Identity auto-detect helper
// ---------------------------------------------------------------------------

/// Try to read `git config --get user.email`. Returns `None` if git is not on
/// PATH or the field is unset.
pub fn detect_identity() -> Option<String> {
    Command::new("git")
        .args(["config", "--get", "user.email"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

// ---------------------------------------------------------------------------
// Paste handler
// ---------------------------------------------------------------------------

/// Translate a pasted string into synthetic [`KeyEvent`]s and feed them into
/// the focused form's text fields.
///
/// Newlines are filtered before building the events — pasting a multi-line
/// string would otherwise trigger Enter (form submit), which is never desired.
pub fn handle_paste(state: &mut OnboardingState, s: &str) {
    let events: Vec<KeyEvent> = s
        .chars()
        .filter(|c| *c != '\r' && *c != '\n')
        .map(|c| KeyEvent {
            code: KeyCode::Char(c),
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        })
        .collect();

    match state {
        OnboardingState::Profile { form } | OnboardingState::Remote { form, .. } => {
            for ev in events {
                form.handle_input(ev);
            }
        }
        OnboardingState::Saving | OnboardingState::Failed(_) => {}
    }
}

// ---------------------------------------------------------------------------
// Save / Skip helpers (pub for tests)
// ---------------------------------------------------------------------------

/// Write a fully-configured user config (profile + remote + active + onboarded).
///
/// Transitions `state` to `Saving` on success or `Failed` on error.
pub fn on_save(
    state: &mut OnboardingState,
    profile: &ProfileFields,
    hub_name: &str,
    hub_url: &str,
    user_config_path: &Path,
) {
    let mut remote_map: BTreeMap<String, RemoteConfig> = BTreeMap::new();
    remote_map.insert(
        hub_name.trim().to_string(),
        RemoteConfig {
            url: hub_url.trim().to_string(),
            default: true,
            provider: None,
            push_mode: quay_core::PushMode::default(),
        },
    );

    let profile_file = ProfileFile {
        user: UserSection {
            name: None,
            email: if profile.identity.is_empty() {
                None
            } else {
                Some(profile.identity.clone())
            },
        },
        remotes: remote_map,
        ..Default::default()
    };

    let mut profiles: BTreeMap<String, ProfileFile> = BTreeMap::new();
    profiles.insert(profile.name.clone(), profile_file);

    let file = UserConfigFile {
        meta: MetaSection { onboarded: true },
        active_profile: Some(profile.name.clone()),
        profiles,
        user: None,
        remotes: None,
    };

    match write_user_file(user_config_path, &file) {
        Ok(()) => *state = OnboardingState::Saving,
        Err(e) => *state = OnboardingState::Failed(e.to_string()),
    }
}

/// Write a config containing only `[meta] onboarded = true`.
pub fn on_skip(user_config_path: &Path) -> Result<(), quay_core::QuayError> {
    let file = UserConfigFile {
        meta: MetaSection { onboarded: true },
        active_profile: None,
        profiles: BTreeMap::new(),
        user: None,
        remotes: None,
    };
    write_user_file(user_config_path, &file)
}

// ---------------------------------------------------------------------------
// Key handler
// ---------------------------------------------------------------------------

/// Handle a key event for the onboarding screen.
pub fn handle_key(app: &mut App, code: KeyCode) -> ScreenAction {
    let placeholder = OnboardingState::default();
    let mut state = std::mem::replace(&mut app.onboarding, placeholder);

    let user_config_path = app.user_config_path.clone();
    let action = handle_key_inner(&mut state, user_config_path.as_deref(), code);

    app.onboarding = state;
    action
}

fn handle_key_inner(
    state: &mut OnboardingState,
    user_config_path: Option<&Path>,
    code: KeyCode,
) -> ScreenAction {
    // Reconstruct a synthetic KeyEvent so we can call form.handle_input.
    // BackTab (Shift+Tab) must be converted to Tab+Shift because the form lib
    // only checks `event.modifiers.contains(SHIFT)` on a Tab keycode.
    let (key_code, modifiers) = if code == KeyCode::BackTab {
        (KeyCode::Tab, KeyModifiers::SHIFT)
    } else {
        (code, KeyModifiers::NONE)
    };
    let key_event = KeyEvent {
        code: key_code,
        modifiers,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    };

    match state {
        OnboardingState::Profile { form } => {
            // Esc on Step 1 = skip onboarding (write meta marker and go to Dashboard).
            if code == KeyCode::Esc {
                if let Some(path) = user_config_path {
                    match on_skip(path) {
                        Ok(()) => *state = OnboardingState::Saving,
                        Err(e) => *state = OnboardingState::Failed(e.to_string()),
                    }
                } else {
                    *state = OnboardingState::Saving;
                }
                return ScreenAction::Stay;
            }

            form.handle_input(key_event);

            if matches!(form.result(), FormResult::Submitted) {
                let json = form.to_json();
                let name = json["name"].as_str().unwrap_or("").trim().to_string();
                let identity = json["identity"].as_str().unwrap_or("").trim().to_string();
                let profile = ProfileFields { name, identity };
                *state = OnboardingState::Remote {
                    profile,
                    form: build_remote_form(),
                };
            }
            // Cancelled (Esc inside form) is already handled above before delegating.
            ScreenAction::Stay
        }

        OnboardingState::Remote { profile, form } => {
            // Esc on Step 2 = back to Step 1.
            if code == KeyCode::Esc {
                let identity = profile.identity.clone();
                *state = OnboardingState::Profile {
                    form: build_profile_form(if identity.is_empty() {
                        None
                    } else {
                        Some(&identity)
                    }),
                };
                return ScreenAction::Stay;
            }

            form.handle_input(key_event);

            if matches!(form.result(), FormResult::Submitted) {
                let json = form.to_json();
                let hub_name = json["hub_name"].as_str().unwrap_or("").to_string();
                let hub_url = json["hub_url"].as_str().unwrap_or("").to_string();
                // Clone profile before calling on_save because on_save mutates state.
                let profile_clone = profile.clone();
                if let Some(path) = user_config_path {
                    on_save(state, &profile_clone, &hub_name, &hub_url, path);
                } else {
                    *state =
                        OnboardingState::Failed("(no config path — use --user-config)".to_string());
                }
            }
            ScreenAction::Stay
        }

        OnboardingState::Saving => ScreenAction::Stay,
        OnboardingState::Failed(_) => {
            // Any key dismisses the error and resets to Step 1.
            *state = OnboardingState::default();
            ScreenAction::Stay
        }
    }
}

// ---------------------------------------------------------------------------
// Render
// ---------------------------------------------------------------------------

/// Render the onboarding screen for the current state.
pub fn render(frame: &mut Frame, _app: &App, area: Rect, state: &OnboardingState) {
    match state {
        OnboardingState::Profile { form } => form.render(area, frame.buffer_mut()),
        OnboardingState::Remote { form, .. } => form.render(area, frame.buffer_mut()),
        OnboardingState::Saving => render_saving(frame, area),
        OnboardingState::Failed(msg) => render_failed(frame, area, msg),
    }
}

fn render_saving(frame: &mut Frame, area: Rect) {
    let outer = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(" Welcome to quay ", theme::accent()));
    let inner = outer.inner(area);
    frame.render_widget(outer, area);
    frame.render_widget(
        Paragraph::new(Span::styled("Saving configuration…", theme::dim())),
        inner,
    );
}

fn render_failed(frame: &mut Frame, area: Rect, msg: &str) {
    let outer = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(" Welcome to quay — Error ", theme::accent()));
    let inner = outer.inner(area);
    frame.render_widget(outer, area);
    let lines = vec![
        Line::from(Span::styled(
            "Failed to save configuration:",
            Style::default().fg(Color::Red),
        )),
        Line::from(Span::raw(msg.to_string())),
        Line::from(""),
        Line::from(Span::styled("(any key to dismiss and retry)", theme::dim())),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use quay_core::{Config, Lockfile};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn fixture_app() -> App {
        App::new(
            Config::default(),
            Lockfile::default(),
            std::path::PathBuf::from("/tmp"),
            None,
        )
    }

    // -- Save writes profile + remote + active to disk --

    #[test]
    fn save_writes_profile_remote_active() {
        let dir = assert_fs::TempDir::new().unwrap();
        let path = dir.path().join("config.toml");

        let profile = ProfileFields {
            name: "p".into(),
            identity: "x@y".into(),
        };
        // Start from a Remote state so on_save transitions from it.
        let mut state2 = OnboardingState::Remote {
            profile: profile.clone(),
            form: build_remote_form(),
        };
        on_save(
            &mut state2,
            &profile,
            "hub",
            "git@example.com:org/skills.git",
            &path,
        );
        assert!(
            matches!(state2, OnboardingState::Saving),
            "state should be Saving after successful write"
        );
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(
            body.contains("active_profile = \"p\""),
            "missing active: {}",
            body
        );
        assert!(
            body.contains(".remotes.hub"),
            "missing hub remote key: {}",
            body
        );
        assert!(
            body.contains("onboarded = true"),
            "missing onboarded: {}",
            body
        );
    }

    // -- Skip writes only meta marker --

    #[test]
    fn skip_writes_only_meta_marker() {
        let dir = assert_fs::TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        on_skip(&path).unwrap();
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(
            body.contains("onboarded = true"),
            "missing marker: {}",
            body
        );
        assert!(
            !body.contains("[profiles."),
            "should not contain profiles: {}",
            body
        );
    }

    // -- Paste fills text fields --

    #[test]
    fn paste_into_step1_name_field() {
        let mut state = OnboardingState::Profile {
            form: build_profile_form(None),
        };
        // Name is the first field (index 0) and starts focused.
        handle_paste(&mut state, "my-profile");
        if let OnboardingState::Profile { form } = &state {
            let json = form.to_json();
            assert_eq!(
                json["name"].as_str().unwrap_or(""),
                "my-profile",
                "paste should fill the focused name field"
            );
        } else {
            panic!("wrong state");
        }
    }

    #[test]
    fn paste_into_step2_url_field_fills_textinput() {
        let mut state = OnboardingState::Remote {
            profile: ProfileFields {
                name: "p".into(),
                identity: "x@y".into(),
            },
            form: build_remote_form(),
        };
        // Tab once to move focus from hub_name to hub_url.
        handle_key_inner(&mut state, None, KeyCode::Tab);
        let events: Vec<KeyEvent> = "git@github.com:o/r.git"
            .chars()
            .map(|c| KeyEvent {
                code: KeyCode::Char(c),
                modifiers: KeyModifiers::NONE,
                kind: KeyEventKind::Press,
                state: KeyEventState::NONE,
            })
            .collect();
        if let OnboardingState::Remote { form, .. } = &mut state {
            for ev in events {
                form.handle_input(ev);
            }
        }
        if let OnboardingState::Remote { form, .. } = &state {
            let json = form.to_json();
            assert_eq!(
                json["hub_url"].as_str().unwrap_or(""),
                "git@github.com:o/r.git"
            );
        } else {
            panic!("wrong state");
        }
    }

    #[test]
    fn paste_drops_newlines() {
        let mut state = OnboardingState::Profile {
            form: build_profile_form(None),
        };
        handle_paste(&mut state, "abc\ndef\rghi");
        if let OnboardingState::Profile { form } = &state {
            let json = form.to_json();
            assert_eq!(json["name"].as_str().unwrap_or(""), "abcdefghi");
        } else {
            panic!("wrong state");
        }
    }

    // -- Render doesn't panic with small area --

    #[test]
    fn render_profile_state_does_not_panic() {
        let mut term = Terminal::new(TestBackend::new(80, 20)).unwrap();
        let app = fixture_app();
        let state = OnboardingState::Profile {
            form: build_profile_form(None),
        };
        term.draw(|f| render(f, &app, f.area(), &state)).unwrap();
    }

    #[test]
    fn render_remote_state_does_not_panic() {
        let mut term = Terminal::new(TestBackend::new(80, 20)).unwrap();
        let app = fixture_app();
        let state = OnboardingState::Remote {
            profile: ProfileFields {
                name: "p".into(),
                identity: "x@y".into(),
            },
            form: build_remote_form(),
        };
        term.draw(|f| render(f, &app, f.area(), &state)).unwrap();
    }
}
