//! First-run onboarding screen — a two-step guided form that runs in place of
//! the dashboard when no user config file exists or `meta.onboarded == false`
//! and no profiles are configured.
//!
//! Step 1: Profile — collect a profile name and identity (email).
//! Step 2: Remote  — collect a hub name and git URL.
//!
//! "Save & continue" persists a fully-configured user config.
//! "Skip" writes a config containing only `meta.onboarded = true`.
//! `Esc` exits without writing anything (onboarding fires again next launch).

use crate::config_io::write_user_file;
use crate::tui::app::{App, ScreenAction};
use crate::tui::theme;
use crossterm::event::KeyCode;
use quay_core::{MetaSection, ProfileFile, RemoteConfig, UserConfigFile, UserSection};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;
use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

// ---------------------------------------------------------------------------
// State types
// ---------------------------------------------------------------------------

/// Focus position within Step 1 (Profile).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PFocus {
    #[default]
    Name,
    Identity,
    Next,
    Skip,
    Cancel,
}

impl PFocus {
    fn next(self) -> PFocus {
        match self {
            PFocus::Name => PFocus::Identity,
            PFocus::Identity => PFocus::Next,
            PFocus::Next => PFocus::Skip,
            PFocus::Skip => PFocus::Cancel,
            PFocus::Cancel => PFocus::Name,
        }
    }

    fn prev(self) -> PFocus {
        match self {
            PFocus::Name => PFocus::Cancel,
            PFocus::Identity => PFocus::Name,
            PFocus::Next => PFocus::Identity,
            PFocus::Skip => PFocus::Next,
            PFocus::Cancel => PFocus::Skip,
        }
    }
}

/// Focus position within Step 2 (Remote).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RFocus {
    #[default]
    Name,
    Url,
    Save,
    Skip,
    Back,
}

impl RFocus {
    fn next(self) -> RFocus {
        match self {
            RFocus::Name => RFocus::Url,
            RFocus::Url => RFocus::Save,
            RFocus::Save => RFocus::Skip,
            RFocus::Skip => RFocus::Back,
            RFocus::Back => RFocus::Name,
        }
    }

    fn prev(self) -> RFocus {
        match self {
            RFocus::Name => RFocus::Back,
            RFocus::Url => RFocus::Name,
            RFocus::Save => RFocus::Url,
            RFocus::Skip => RFocus::Save,
            RFocus::Back => RFocus::Skip,
        }
    }
}

/// Captured profile data carried from Step 1 into Step 2.
#[derive(Debug, Clone)]
pub struct ProfileFields {
    pub name: String,
    pub identity: String,
}

/// Inline validation hints shown next to fields on Step 1.
#[derive(Debug, Clone, Default)]
pub struct StepOneHint {
    pub name_error: Option<String>,
    pub identity_error: Option<String>,
}

/// Full onboarding state machine.
#[derive(Debug)]
pub enum OnboardingState {
    /// Step 1: collect profile name + identity.
    Profile {
        name: String,
        identity: String,
        focus: PFocus,
        hint: StepOneHint,
    },
    /// Step 2: collect remote hub name + git URL.
    Remote {
        profile: ProfileFields,
        hub_name: String,
        url: String,
        focus: RFocus,
        url_error: Option<String>,
    },
    /// A write completed successfully; the app loop will switch to Dashboard.
    Saving,
    /// A write failed.
    Failed(String),
}

impl Default for OnboardingState {
    fn default() -> Self {
        let identity = detect_identity().unwrap_or_default();
        OnboardingState::Profile {
            name: String::new(),
            identity,
            focus: PFocus::Name,
            hint: StepOneHint::default(),
        }
    }
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
// Save / Skip helpers (pub for tests)
// ---------------------------------------------------------------------------

/// Write a fully-configured user config (profile + remote + active + onboarded).
/// Transitions `state` to `Saving` on success or `Failed` on error.
pub fn on_save(state: &mut OnboardingState, user_config_path: &Path) {
    let (profile, hub_name, url) = match state {
        OnboardingState::Remote {
            profile,
            hub_name,
            url,
            ..
        } => (
            profile.clone(),
            hub_name.trim().to_string(),
            url.trim().to_string(),
        ),
        _ => return,
    };

    let mut remote_map: BTreeMap<String, RemoteConfig> = BTreeMap::new();
    remote_map.insert(
        hub_name.clone(),
        RemoteConfig {
            url,
            default: true,
            provider: None,
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
// Validation helpers
// ---------------------------------------------------------------------------

fn is_valid_profile_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

// ---------------------------------------------------------------------------
// Key handler
// ---------------------------------------------------------------------------

/// Handle a key event for the onboarding screen.
pub fn handle_key(app: &mut App, code: KeyCode) -> ScreenAction {
    // Move state out to avoid aliasing.
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
    match state {
        OnboardingState::Profile { .. } => handle_step1(state, user_config_path, code),
        OnboardingState::Remote { .. } => handle_step2(state, user_config_path, code),
        OnboardingState::Saving => ScreenAction::Stay,
        OnboardingState::Failed(_) => {
            // Any key dismisses the error and resets to Step 1.
            *state = OnboardingState::default();
            ScreenAction::Stay
        }
    }
}

fn handle_step1(
    state: &mut OnboardingState,
    user_config_path: Option<&Path>,
    code: KeyCode,
) -> ScreenAction {
    let focus = match state {
        OnboardingState::Profile { focus, .. } => *focus,
        _ => return ScreenAction::Stay,
    };

    match code {
        KeyCode::Esc => {
            return ScreenAction::Quit;
        }
        KeyCode::Tab => {
            if let OnboardingState::Profile { focus, .. } = state {
                *focus = focus.next();
            }
        }
        KeyCode::BackTab => {
            if let OnboardingState::Profile { focus, .. } = state {
                *focus = focus.prev();
            }
        }
        KeyCode::Backspace => {
            if let OnboardingState::Profile {
                focus,
                name,
                identity,
                ..
            } = state
            {
                match focus {
                    PFocus::Name => {
                        name.pop();
                    }
                    PFocus::Identity => {
                        identity.pop();
                    }
                    _ => {}
                }
            }
        }
        KeyCode::Char(c) => {
            if let OnboardingState::Profile {
                focus,
                name,
                identity,
                ..
            } = state
            {
                match focus {
                    PFocus::Name => name.push(c),
                    PFocus::Identity => identity.push(c),
                    _ => {}
                }
            }
        }
        KeyCode::Enter => match focus {
            PFocus::Skip => {
                // Skip from Step 1: write the onboarded marker.
                if let Some(path) = user_config_path {
                    match on_skip(path) {
                        Ok(()) => *state = OnboardingState::Saving,
                        Err(e) => *state = OnboardingState::Failed(e.to_string()),
                    }
                } else {
                    // No config path: just mark as saving (user has no config file).
                    *state = OnboardingState::Saving;
                }
            }
            PFocus::Cancel => {
                return ScreenAction::Quit;
            }
            _ => {
                try_advance_to_step2(state);
            }
        },
        _ => {}
    }

    ScreenAction::Stay
}

/// Validate Step 1 inputs and advance to Step 2, or set hints on failure.
pub fn try_advance_to_step2(state: &mut OnboardingState) {
    let (name, identity) = match state {
        OnboardingState::Profile { name, identity, .. } => {
            (name.trim().to_string(), identity.trim().to_string())
        }
        _ => return,
    };

    let mut hint = StepOneHint::default();
    if !is_valid_profile_name(&name) {
        hint.name_error = Some(
            if name.is_empty() {
                "(required)"
            } else {
                "(lowercase letters, digits, hyphens only)"
            }
            .into(),
        );
    }
    if identity.is_empty() {
        hint.identity_error = Some("(required)".into());
    }

    if hint.name_error.is_some() || hint.identity_error.is_some() {
        let focus_to = if hint.name_error.is_some() {
            PFocus::Name
        } else {
            PFocus::Identity
        };
        if let OnboardingState::Profile { focus, hint: h, .. } = state {
            *focus = focus_to;
            *h = hint;
        }
        return;
    }

    *state = OnboardingState::Remote {
        profile: ProfileFields { name, identity },
        hub_name: String::new(),
        url: String::new(),
        focus: RFocus::Name,
        url_error: None,
    };
}

fn handle_step2(
    state: &mut OnboardingState,
    user_config_path: Option<&Path>,
    code: KeyCode,
) -> ScreenAction {
    let focus = match state {
        OnboardingState::Remote { focus, .. } => *focus,
        _ => return ScreenAction::Stay,
    };

    match code {
        KeyCode::Esc => {
            // Back to Step 1.
            if let OnboardingState::Remote { profile, .. } = state {
                let identity = profile.identity.clone();
                let name = profile.name.clone();
                *state = OnboardingState::Profile {
                    name,
                    identity,
                    focus: PFocus::Name,
                    hint: StepOneHint::default(),
                };
            }
        }
        KeyCode::Tab => {
            if let OnboardingState::Remote { focus, .. } = state {
                *focus = focus.next();
            }
        }
        KeyCode::BackTab => {
            if let OnboardingState::Remote { focus, .. } = state {
                *focus = focus.prev();
            }
        }
        KeyCode::Backspace => {
            if let OnboardingState::Remote {
                focus,
                hub_name,
                url,
                ..
            } = state
            {
                match focus {
                    RFocus::Name => {
                        hub_name.pop();
                    }
                    RFocus::Url => {
                        url.pop();
                    }
                    _ => {}
                }
            }
        }
        KeyCode::Char(c) => {
            if let OnboardingState::Remote {
                focus,
                hub_name,
                url,
                ..
            } = state
            {
                match focus {
                    RFocus::Name => hub_name.push(c),
                    RFocus::Url => url.push(c),
                    _ => {}
                }
            }
        }
        KeyCode::Enter => match focus {
            RFocus::Save => {
                let url_empty = match state {
                    OnboardingState::Remote { url, .. } => url.trim().is_empty(),
                    _ => false,
                };
                if url_empty {
                    if let OnboardingState::Remote { url_error, .. } = state {
                        *url_error = Some("(required)".into());
                    }
                } else if let Some(path) = user_config_path {
                    on_save(state, path);
                } else if let OnboardingState::Remote { url_error, .. } = state {
                    *url_error = Some("(no config path — use --user-config)".into());
                }
            }
            RFocus::Skip => {
                if let Some(path) = user_config_path {
                    match on_skip(path) {
                        Ok(()) => *state = OnboardingState::Saving,
                        Err(e) => *state = OnboardingState::Failed(e.to_string()),
                    }
                } else {
                    *state = OnboardingState::Saving;
                }
            }
            RFocus::Back => {
                if let OnboardingState::Remote { profile, .. } = state {
                    let identity = profile.identity.clone();
                    let name = profile.name.clone();
                    *state = OnboardingState::Profile {
                        name,
                        identity,
                        focus: PFocus::Name,
                        hint: StepOneHint::default(),
                    };
                }
            }
            _ => {
                // Advance focus on Enter in text fields.
                if let OnboardingState::Remote { focus, .. } = state {
                    *focus = focus.next();
                }
            }
        },
        _ => {}
    }

    ScreenAction::Stay
}

// ---------------------------------------------------------------------------
// Render
// ---------------------------------------------------------------------------

/// Render the onboarding screen for the current state.
pub fn render(frame: &mut Frame, _app: &App, area: Rect, state: &OnboardingState) {
    match state {
        OnboardingState::Profile {
            name,
            identity,
            focus,
            hint,
        } => render_step1(frame, area, name, identity, *focus, hint),
        OnboardingState::Remote {
            profile,
            hub_name,
            url,
            focus,
            url_error,
        } => render_step2(frame, area, profile, hub_name, url, *focus, url_error),
        OnboardingState::Saving => render_saving(frame, area),
        OnboardingState::Failed(msg) => render_failed(frame, area, msg),
    }
}

fn render_step1(
    frame: &mut Frame,
    area: Rect,
    name: &str,
    identity: &str,
    focus: PFocus,
    hint: &StepOneHint,
) {
    let outer = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(" Welcome to quay ", theme::accent()));
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // blank
            Constraint::Length(1), // "No profiles configured"
            Constraint::Length(1), // blank
            Constraint::Length(1), // "Step 1 of 2"
            Constraint::Length(2), // Name field
            Constraint::Length(2), // Identity field
            Constraint::Length(1), // blank
            Constraint::Length(1), // buttons
            Constraint::Length(1), // footer
        ])
        .split(inner);

    frame.render_widget(
        Paragraph::new(Span::styled("No profiles configured yet.", theme::dim())),
        rows[1],
    );

    frame.render_widget(
        Paragraph::new(Span::styled(
            "Step 1 of 2: Create your first profile",
            theme::accent(),
        )),
        rows[3],
    );

    render_onboarding_field(
        frame,
        rows[4],
        "Name",
        name,
        hint.name_error.as_deref(),
        focus == PFocus::Name,
    );
    render_onboarding_field(
        frame,
        rows[5],
        "Identity (email)",
        identity,
        hint.identity_error.as_deref(),
        focus == PFocus::Identity,
    );

    let next_style = if focus == PFocus::Next {
        theme::selected()
    } else {
        Style::default()
    };
    let skip_style = if focus == PFocus::Skip {
        theme::selected()
    } else {
        Style::default()
    };
    let cancel_style = if focus == PFocus::Cancel {
        theme::selected()
    } else {
        Style::default()
    };

    let buttons = Line::from(vec![
        Span::styled(" [ Next → ] ", next_style),
        Span::raw("   "),
        Span::styled(" [ Skip — I'll configure later ] ", skip_style),
        Span::raw("   "),
        Span::styled(" [ Cancel ] ", cancel_style),
    ]);
    frame.render_widget(Paragraph::new(buttons), rows[7]);

    frame.render_widget(
        Paragraph::new(Span::styled(
            "Tab — cycle fields   Enter — next/select   Esc — quit",
            theme::dim(),
        )),
        rows[8],
    );
}

fn render_step2(
    frame: &mut Frame,
    area: Rect,
    profile: &ProfileFields,
    hub_name: &str,
    url: &str,
    focus: RFocus,
    url_error: &Option<String>,
) {
    let outer = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(" Welcome to quay ", theme::accent()));
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // blank
            Constraint::Length(1), // profile summary
            Constraint::Length(1), // "Step 2 of 2"
            Constraint::Length(2), // Hub name field
            Constraint::Length(2), // URL field
            Constraint::Length(1), // blank
            Constraint::Length(1), // buttons
            Constraint::Length(1), // footer
        ])
        .split(inner);

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Profile: ", theme::dim()),
            Span::styled(profile.name.clone(), theme::accent()),
            Span::styled("  Identity: ", theme::dim()),
            Span::styled(profile.identity.clone(), theme::accent()),
        ])),
        rows[1],
    );

    frame.render_widget(
        Paragraph::new(Span::styled(
            "Step 2 of 2: Add a remote skill hub",
            theme::accent(),
        )),
        rows[2],
    );

    render_onboarding_field(
        frame,
        rows[3],
        "Hub name",
        hub_name,
        None,
        focus == RFocus::Name,
    );
    render_onboarding_field(
        frame,
        rows[4],
        "Git URL",
        url,
        url_error.as_deref(),
        focus == RFocus::Url,
    );

    let save_style = if focus == RFocus::Save {
        theme::selected()
    } else {
        Style::default()
    };
    let skip_style = if focus == RFocus::Skip {
        theme::selected()
    } else {
        Style::default()
    };
    let back_style = if focus == RFocus::Back {
        theme::selected()
    } else {
        Style::default()
    };

    let buttons = Line::from(vec![
        Span::styled(" [ ← Back ] ", back_style),
        Span::raw("   "),
        Span::styled(" [ Save & continue ] ", save_style),
        Span::raw("   "),
        Span::styled(" [ Skip ] ", skip_style),
    ]);
    frame.render_widget(Paragraph::new(buttons), rows[6]);

    frame.render_widget(
        Paragraph::new(Span::styled(
            "Tab — cycle fields   Enter — save/select   Esc — back",
            theme::dim(),
        )),
        rows[7],
    );
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

fn render_onboarding_field(
    frame: &mut Frame,
    area: Rect,
    label: &str,
    value: &str,
    error: Option<&str>,
    focused: bool,
) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(area);

    let label_style = if focused {
        theme::accent()
    } else {
        theme::dim()
    };
    let cursor = if focused { "▶ " } else { "  " };

    let mut label_spans = vec![
        Span::styled(cursor, label_style),
        Span::styled(label.to_string(), label_style),
    ];
    if let Some(err) = error {
        label_spans.push(Span::raw("  "));
        label_spans.push(Span::styled(
            err.to_string(),
            Style::default().fg(Color::Red),
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(label_spans)), rows[0]);

    let value_style = if focused {
        Style::default().fg(Color::White)
    } else {
        Style::default()
    };
    frame.render_widget(
        Paragraph::new(Span::styled(format!("  {}", value), value_style)),
        rows[1],
    );
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

    fn buf_contains(buf: &ratatui::buffer::Buffer, needle: &str) -> bool {
        let content: String = buf.content().iter().map(|c| c.symbol()).collect();
        content.contains(needle)
    }

    fn fixture_app() -> App {
        App::new(
            Config::default(),
            Lockfile::default(),
            std::path::PathBuf::from("/tmp"),
            None,
        )
    }

    // -- Step 1 snapshot --

    #[test]
    fn step1_renders_key_elements() {
        let mut term = Terminal::new(TestBackend::new(80, 20)).unwrap();
        let app = fixture_app();
        let state = OnboardingState::Profile {
            name: String::new(),
            identity: String::new(),
            focus: PFocus::Name,
            hint: StepOneHint::default(),
        };
        term.draw(|f| render(f, &app, f.area(), &state)).unwrap();
        let buf = term.backend().buffer().clone();
        assert!(buf_contains(&buf, "Welcome to quay"), "missing title");
        assert!(buf_contains(&buf, "Step 1 of 2"), "missing step header");
        assert!(buf_contains(&buf, "Name"), "missing Name field");
        assert!(buf_contains(&buf, "Identity"), "missing Identity field");
        assert!(buf_contains(&buf, "Next"), "missing Next button");
        assert!(buf_contains(&buf, "Skip"), "missing Skip button");
    }

    // -- Focus cycling --

    #[test]
    fn pfocus_tab_cycles_name_identity_next_skip_cancel() {
        let mut focus = PFocus::Name;
        focus = focus.next();
        assert!(matches!(focus, PFocus::Identity));
        focus = focus.next();
        assert!(matches!(focus, PFocus::Next));
        focus = focus.next();
        assert!(matches!(focus, PFocus::Skip));
        focus = focus.next();
        assert!(matches!(focus, PFocus::Cancel));
        focus = focus.next();
        assert!(matches!(focus, PFocus::Name));
    }

    #[test]
    fn rfocus_tab_cycles_name_url_save_skip_back() {
        let mut focus = RFocus::Name;
        focus = focus.next();
        assert!(matches!(focus, RFocus::Url));
        focus = focus.next();
        assert!(matches!(focus, RFocus::Save));
        focus = focus.next();
        assert!(matches!(focus, RFocus::Skip));
        focus = focus.next();
        assert!(matches!(focus, RFocus::Back));
        focus = focus.next();
        assert!(matches!(focus, RFocus::Name));
    }

    // -- Validation: empty name stays on Step 1 --

    #[test]
    fn next_on_empty_name_stays_on_step1() {
        let mut state = OnboardingState::Profile {
            name: String::new(),
            identity: "someone@example.com".into(),
            focus: PFocus::Next,
            hint: StepOneHint::default(),
        };
        try_advance_to_step2(&mut state);
        assert!(
            matches!(state, OnboardingState::Profile { .. }),
            "should remain on Step 1"
        );
        if let OnboardingState::Profile { hint, .. } = &state {
            assert!(hint.name_error.is_some(), "should show name error");
        }
    }

    #[test]
    fn next_on_invalid_name_stays_on_step1() {
        let mut state = OnboardingState::Profile {
            name: "BadName".into(),
            identity: "someone@example.com".into(),
            focus: PFocus::Next,
            hint: StepOneHint::default(),
        };
        try_advance_to_step2(&mut state);
        assert!(matches!(state, OnboardingState::Profile { .. }));
    }

    #[test]
    fn next_on_valid_inputs_advances_to_step2() {
        let mut state = OnboardingState::Profile {
            name: "personal".into(),
            identity: "me@example.com".into(),
            focus: PFocus::Next,
            hint: StepOneHint::default(),
        };
        try_advance_to_step2(&mut state);
        assert!(
            matches!(state, OnboardingState::Remote { .. }),
            "should advance to Step 2"
        );
    }

    // -- Step 2 snapshot --

    #[test]
    fn step2_renders_key_elements() {
        let mut term = Terminal::new(TestBackend::new(80, 20)).unwrap();
        let app = fixture_app();
        let state = OnboardingState::Remote {
            profile: ProfileFields {
                name: "personal".into(),
                identity: "me@example.com".into(),
            },
            hub_name: String::new(),
            url: String::new(),
            focus: RFocus::Name,
            url_error: None,
        };
        term.draw(|f| render(f, &app, f.area(), &state)).unwrap();
        let buf = term.backend().buffer().clone();
        assert!(buf_contains(&buf, "Step 2 of 2"), "missing step 2 header");
        assert!(buf_contains(&buf, "Hub name"), "missing Hub name field");
        assert!(buf_contains(&buf, "Git URL"), "missing Git URL field");
        assert!(buf_contains(&buf, "Save"), "missing Save button");
    }

    // -- Save writes profile + remote + active to disk --

    #[test]
    fn save_writes_profile_remote_active() {
        let dir = assert_fs::TempDir::new().unwrap();
        let path = dir.path().join("config.toml");

        let mut state = OnboardingState::Remote {
            profile: ProfileFields {
                name: "p".into(),
                identity: "x@y".into(),
            },
            hub_name: "hub".into(),
            url: "git@example.com:org/skills.git".into(),
            focus: RFocus::Save,
            url_error: None,
        };
        on_save(&mut state, &path);
        assert!(
            matches!(state, OnboardingState::Saving),
            "state should be Saving after successful write"
        );
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(
            body.contains("active_profile = \"p\""),
            "missing active: {}",
            body
        );
        // The hub name "hub" is serialized as the BTreeMap key in TOML, e.g.
        // `[profiles.p.remotes.hub]`, not as a `name = "hub"` field.
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

    // -- Tab key cycles focus via handle_key (step 1) --

    #[test]
    fn tab_key_cycles_focus_in_step1() {
        let mut app = fixture_app();
        app.onboarding = OnboardingState::Profile {
            name: String::new(),
            identity: String::new(),
            focus: PFocus::Name,
            hint: StepOneHint::default(),
        };
        handle_key(&mut app, KeyCode::Tab);
        assert!(
            matches!(
                app.onboarding,
                OnboardingState::Profile {
                    focus: PFocus::Identity,
                    ..
                }
            ),
            "Tab should advance focus to Identity"
        );
    }
}
