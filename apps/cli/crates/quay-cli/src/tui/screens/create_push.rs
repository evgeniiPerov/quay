//! Screen 5 — Create / Push.
//!
//! State machine:
//! `Form` → `ScaffoldRunning` → `Editing` → `ReadyToValidate` → `Validating`
//!   → `ValidateErrors` | `ReadyToPush` → `Pushing` → `Done`
//!
//! Error recovery: `Failed { state, message }` renders a banner and transitions
//! back to the prior stable state on any key press.

use crate::commands;
use crate::tui::app::{App, BlockingAction, ScreenAction};
use crate::tui::screens::widgets::spinner::Spinner;
use crate::tui::theme;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use quay_core::BumpKind;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph};
use ratatui::Frame;
use ratatui_form::{Form, FormResult};
use std::path::PathBuf;
use std::time::Instant;

use crate::commands::push::PushOutcome;

/// Which bump level to apply on push.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BumpChoice {
    #[default]
    Patch,
    Minor,
    Major,
    AsWritten,
}

impl BumpChoice {
    fn as_bump_kind(self) -> BumpKind {
        match self {
            BumpChoice::Patch => BumpKind::Patch,
            BumpChoice::Minor => BumpKind::Minor,
            BumpChoice::Major => BumpKind::Major,
            BumpChoice::AsWritten => BumpKind::AsWritten,
        }
    }

    fn label(self) -> &'static str {
        match self {
            BumpChoice::Patch => "patch",
            BumpChoice::Minor => "minor",
            BumpChoice::Major => "major",
            BumpChoice::AsWritten => "as-written",
        }
    }

    fn next(self) -> BumpChoice {
        match self {
            BumpChoice::Patch => BumpChoice::Minor,
            BumpChoice::Minor => BumpChoice::Major,
            BumpChoice::Major => BumpChoice::AsWritten,
            BumpChoice::AsWritten => BumpChoice::Patch,
        }
    }
}

// ---------------------------------------------------------------------------
// Form builder
// ---------------------------------------------------------------------------

/// Build the Create Skill frontmatter form.
///
/// If `remotes` is non-empty, a `Select` field is appended for the target
/// remote.  If it is empty, the select field is omitted.
pub fn build_create_form(remotes: &[String]) -> Form {
    let mut b = Form::builder()
        .title("Create Skill")
        .style(crate::tui::form_theme::dark())
        .text("name", "Name")
        .required()
        .validator(Box::new(ratatui_form::Pattern::new(
            r"^[a-z0-9-]+$",
            "kebab-case only (lowercase letters, digits, hyphens)",
        )))
        .done()
        .text("description", "Description")
        .required()
        .done()
        .text("tags", "Tags (comma-separated)")
        .done();
    if !remotes.is_empty() {
        let options: Vec<(&str, &str)> = remotes.iter().map(|s| (s.as_str(), s.as_str())).collect();
        b = b
            .select("remote", "Target remote")
            .options(options)
            .initial_value(remotes[0].as_str())
            .done();
    }
    b.build()
}

/// Build a fresh form pre-populated from the app's current config remotes.
pub fn build_create_form_from_app(app: &App) -> Form {
    let remotes: Vec<String> = app.cfg.remotes.keys().cloned().collect();
    build_create_form(&remotes)
}

// ---------------------------------------------------------------------------
// State machine
// ---------------------------------------------------------------------------

/// Full state machine for Screen 5.
pub enum CreatePushState {
    /// Collecting frontmatter from the user via `ratatui-form`.
    Form(ratatui_form::Form),
    /// Scaffold is running (fast disk write — this state is mostly a visual
    /// marker; we transition through it synchronously before entering `Editing`).
    ScaffoldRunning,
    /// `$EDITOR` is open for the skill body (TUI is suspended).
    Editing { skill: String, path: PathBuf },
    /// The editor has exited; waiting for the user to trigger validation.
    ReadyToValidate { skill: String, path: PathBuf },
    /// Validation is running (fast, synchronous).
    Validating { skill: String, path: PathBuf },
    /// Validation produced errors.
    ValidateErrors {
        skill: String,
        path: PathBuf,
        errors: Vec<String>,
    },
    /// Validation passed; the user can inspect and confirm the push.
    ReadyToPush {
        skill: String,
        path: PathBuf,
        remote: Option<String>,
        bump: BumpChoice,
    },
    /// Push is in progress; spinner animates on each tick.
    Pushing {
        skill: String,
        remote: Option<String>,
        bump: BumpChoice,
        started_at: Instant,
        spinner: Spinner,
    },
    /// Push succeeded.
    Done(PushOutcome),
    /// An operation failed; `message` is shown as a banner.  The boxed `state`
    /// is the prior stable state we will return to on acknowledgement.
    Failed {
        state: Box<CreatePushState>,
        message: String,
    },
}

impl std::fmt::Debug for CreatePushState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CreatePushState::Form(_) => write!(f, "Form(...)"),
            CreatePushState::ScaffoldRunning => write!(f, "ScaffoldRunning"),
            CreatePushState::Editing { skill, path } => f
                .debug_struct("Editing")
                .field("skill", skill)
                .field("path", path)
                .finish(),
            CreatePushState::ReadyToValidate { skill, path } => f
                .debug_struct("ReadyToValidate")
                .field("skill", skill)
                .field("path", path)
                .finish(),
            CreatePushState::Validating { skill, path } => f
                .debug_struct("Validating")
                .field("skill", skill)
                .field("path", path)
                .finish(),
            CreatePushState::ValidateErrors {
                skill,
                path,
                errors,
            } => f
                .debug_struct("ValidateErrors")
                .field("skill", skill)
                .field("path", path)
                .field("errors", errors)
                .finish(),
            CreatePushState::ReadyToPush {
                skill,
                path,
                remote,
                bump,
            } => f
                .debug_struct("ReadyToPush")
                .field("skill", skill)
                .field("path", path)
                .field("remote", remote)
                .field("bump", bump)
                .finish(),
            CreatePushState::Pushing {
                skill,
                remote,
                bump,
                ..
            } => f
                .debug_struct("Pushing")
                .field("skill", skill)
                .field("remote", remote)
                .field("bump", bump)
                .finish(),
            CreatePushState::Done(o) => write!(f, "Done({:?})", o),
            CreatePushState::Failed { message, .. } => {
                f.debug_struct("Failed").field("message", message).finish()
            }
        }
    }
}

impl CreatePushState {
    /// Advance the spinner if we are in the `Pushing` state.
    pub fn tick(&mut self) {
        if let CreatePushState::Pushing { spinner, .. } = self {
            spinner.advance();
        }
    }
}

// ---------------------------------------------------------------------------
// Paste handler
// ---------------------------------------------------------------------------

/// Insert a pasted string into the currently focused text field of the form.
///
/// Only the `Form` state accepts paste; all other states in the state machine
/// (scaffold running, editing, validating, etc.) silently drop the paste.
pub fn handle_paste(state: &mut CreatePushState, s: &str) {
    if let CreatePushState::Form(form) = state {
        let events = crate::tui::paste_to_key_events(s);
        for ev in events {
            form.handle_input(ev);
        }
    }
}

// ---------------------------------------------------------------------------
// Key handler
// ---------------------------------------------------------------------------

/// Handle a key event for the create/push screen.  Returns the desired
/// [`ScreenAction`] (stay, switch, or quit).
///
/// Temporarily moves `app.create_push` out via [`std::mem::replace`] so we can
/// hold `&mut App` (for side-effects such as `set_status` / `defer_blocking_action`)
/// without aliasing `app.create_push`.  The state is moved back at the end.
pub fn handle_key(app: &mut App, code: KeyCode) -> ScreenAction {
    // Move state out to avoid aliasing.
    let placeholder = CreatePushState::Form(build_create_form(&[]));
    let mut state = std::mem::replace(&mut app.create_push, placeholder);

    let action = handle_key_inner(&mut state, app, code);

    // Move state back.
    app.create_push = state;
    action
}

fn handle_key_inner(state: &mut CreatePushState, app: &mut App, code: KeyCode) -> ScreenAction {
    let discriminant = state_discriminant(state);
    match discriminant {
        StateKind::Form => handle_form(state, app, code),
        StateKind::ReadyToValidate => handle_ready_to_validate(state, app, code),
        StateKind::ValidateErrors => handle_validate_errors(state, app, code),
        StateKind::ReadyToPush => handle_ready_to_push(state, app, code),
        StateKind::Done => handle_done(state, app, code),
        StateKind::Failed => {
            // Any key dismisses the failure banner. If the prior state was
            // Pushing (a transient state with no key handlers), unwrapping
            // to it would leave the user stuck on a frozen spinner — instead
            // bail back to the Dashboard so they can retry from there.
            let was_after_pushing = matches!(
                state,
                CreatePushState::Failed { state: prior, .. }
                    if matches!(**prior, CreatePushState::Pushing { .. })
            );
            dismiss_failure(state, app);
            if was_after_pushing {
                return ScreenAction::SwitchTo(crate::tui::app::Screen::Dashboard);
            }
            ScreenAction::Stay
        }
        StateKind::Other => ScreenAction::Stay,
    }
}

/// Discriminant-only copy so we can branch without holding a live borrow.
#[derive(Debug, Clone, Copy)]
enum StateKind {
    Form,
    ReadyToValidate,
    ValidateErrors,
    ReadyToPush,
    Done,
    Failed,
    Other,
}

fn state_discriminant(state: &CreatePushState) -> StateKind {
    match state {
        CreatePushState::Form(_) => StateKind::Form,
        CreatePushState::ReadyToValidate { .. } => StateKind::ReadyToValidate,
        CreatePushState::ValidateErrors { .. } => StateKind::ValidateErrors,
        CreatePushState::ReadyToPush { .. } => StateKind::ReadyToPush,
        CreatePushState::Done(_) => StateKind::Done,
        CreatePushState::Failed { .. } => StateKind::Failed,
        _ => StateKind::Other,
    }
}

fn handle_form(state: &mut CreatePushState, app: &mut App, code: KeyCode) -> ScreenAction {
    // Intercept Esc before delegating to the form — Esc cancels to Dashboard.
    if code == KeyCode::Esc {
        return ScreenAction::SwitchTo(crate::tui::app::Screen::Dashboard);
    }

    // Translate BackTab → Tab + SHIFT (ratatui-form checks modifiers).
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

    if let CreatePushState::Form(form) = state {
        form.handle_input(key_event);

        match form.result() {
            FormResult::Submitted => {
                let json = form.to_json();
                let name = json["name"].as_str().unwrap_or("").trim().to_string();
                let _description = json["description"]
                    .as_str()
                    .unwrap_or("")
                    .trim()
                    .to_string();
                let _tags = json["tags"].as_str().unwrap_or("").trim().to_string();
                let _remote = json
                    .get("remote")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());

                if name.is_empty() {
                    app.set_status("skill name is required");
                    return ScreenAction::Stay;
                }

                on_save(state, app, &name);
            }
            FormResult::Cancelled => {
                return ScreenAction::SwitchTo(crate::tui::app::Screen::Dashboard);
            }
            FormResult::Active => {}
        }
    }
    ScreenAction::Stay
}

fn handle_ready_to_validate(
    state: &mut CreatePushState,
    app: &mut App,
    code: KeyCode,
) -> ScreenAction {
    match code {
        KeyCode::Char('v') | KeyCode::Enter => {
            on_validate(state, app);
        }
        KeyCode::Char('e') => {
            re_open_editor(state);
        }
        KeyCode::Esc | KeyCode::Char('c') | KeyCode::Char('q') => {
            return ScreenAction::SwitchTo(crate::tui::app::Screen::Dashboard);
        }
        _ => {}
    }
    ScreenAction::Stay
}

fn handle_validate_errors(
    state: &mut CreatePushState,
    _app: &mut App,
    code: KeyCode,
) -> ScreenAction {
    match code {
        KeyCode::Char('e') | KeyCode::Enter => {
            re_open_editor_from_errors(state);
        }
        KeyCode::Esc | KeyCode::Char('c') | KeyCode::Char('q') => {
            return ScreenAction::SwitchTo(crate::tui::app::Screen::Dashboard);
        }
        _ => {}
    }
    ScreenAction::Stay
}

fn handle_ready_to_push(state: &mut CreatePushState, app: &mut App, code: KeyCode) -> ScreenAction {
    match code {
        KeyCode::Char('b') | KeyCode::Tab => {
            if let CreatePushState::ReadyToPush { bump, .. } = state {
                *bump = bump.next();
            }
        }
        KeyCode::Char('p') | KeyCode::Enter => {
            on_push(state, app);
        }
        KeyCode::Esc | KeyCode::Char('c') => {
            return ScreenAction::SwitchTo(crate::tui::app::Screen::Dashboard);
        }
        _ => {}
    }
    ScreenAction::Stay
}

// In tests, avoid actually spawning the system browser.
#[cfg(not(test))]
fn open_url(url: &str) -> std::io::Result<()> {
    crate::url_opener::open_browser(url)
}
#[cfg(test)]
fn open_url(url: &str) -> std::io::Result<()> {
    crate::url_opener::open_browser_with(url, crate::url_opener::OpenStrategy::Stub)
}

fn handle_done(state: &mut CreatePushState, app: &mut App, code: KeyCode) -> ScreenAction {
    match code {
        KeyCode::Char('o') => {
            if let CreatePushState::Done(outcome) = state {
                if outcome.pr_url.is_empty() {
                    // Direct-mode push: no PR URL to open. Provider-specific commit URL is
                    // out of scope for this plan — surface a helpful status instead.
                    app.set_status("direct push: no PR URL (commit on hub default branch)");
                } else {
                    match open_url(&outcome.pr_url) {
                        Ok(()) => app.set_status(format!("opened: {}", outcome.pr_url)),
                        Err(e) => {
                            app.set_status(format!("open failed: {} (url: {})", e, outcome.pr_url))
                        }
                    }
                }
            }
        }
        KeyCode::Char('b') | KeyCode::Esc => {
            return ScreenAction::SwitchTo(crate::tui::app::Screen::Dashboard);
        }
        _ => {}
    }
    ScreenAction::Stay
}

// ---------------------------------------------------------------------------
// State transition helpers
// ---------------------------------------------------------------------------

fn on_save(state: &mut CreatePushState, app: &mut App, name: &str) {
    match commands::create::scaffold(
        name,
        None,
        app.cfg.user.email.as_deref(),
        &app.project_root,
        app.user_config_path.as_deref(),
    ) {
        Ok(outcome) => {
            // Suspend TUI and open editor.
            match crate::tui::editor::run_editor(&outcome.skill_md_path) {
                Ok(()) => {
                    *state = CreatePushState::ReadyToValidate {
                        skill: outcome.skill,
                        path: outcome.skill_md_path,
                    };
                }
                Err(e) => {
                    *state = CreatePushState::Failed {
                        state: Box::new(CreatePushState::Form(build_create_form_from_app(app))),
                        message: format!("editor: {}", e),
                    };
                }
            }
        }
        Err(e) => {
            *state = CreatePushState::Failed {
                state: Box::new(CreatePushState::Form(build_create_form_from_app(app))),
                message: e.to_string(),
            };
        }
    }
}

fn on_validate(state: &mut CreatePushState, app: &mut App) {
    let (skill, path) = match state {
        CreatePushState::ReadyToValidate { skill, path } => (skill.clone(), path.clone()),
        _ => return,
    };

    match commands::validate::validate_skill(
        &skill,
        &app.project_root,
        commands::validate::ValidateMode::Strict,
    ) {
        Ok(outcome) if outcome.warnings.is_empty() => {
            let remote = app.cfg.default_remote().map(|(name, _)| name.clone());
            *state = CreatePushState::ReadyToPush {
                skill,
                path,
                remote,
                bump: BumpChoice::default(),
            };
        }
        Ok(outcome) => {
            *state = CreatePushState::ValidateErrors {
                skill,
                path,
                errors: outcome.warnings,
            };
        }
        Err(e) => {
            let prior = CreatePushState::ReadyToValidate { skill, path };
            *state = CreatePushState::Failed {
                state: Box::new(prior),
                message: e.to_string(),
            };
        }
    }
}

fn re_open_editor(state: &mut CreatePushState) {
    let (skill, path) = match state {
        CreatePushState::ReadyToValidate { skill, path } => (skill.clone(), path.clone()),
        _ => return,
    };
    match crate::tui::editor::run_editor(&path) {
        Ok(()) => {
            *state = CreatePushState::ReadyToValidate { skill, path };
        }
        Err(e) => {
            let prior = CreatePushState::ReadyToValidate { skill, path };
            *state = CreatePushState::Failed {
                state: Box::new(prior),
                message: format!("editor: {}", e),
            };
        }
    }
}

fn re_open_editor_from_errors(state: &mut CreatePushState) {
    let (skill, path) = match state {
        CreatePushState::ValidateErrors { skill, path, .. } => (skill.clone(), path.clone()),
        _ => return,
    };
    match crate::tui::editor::run_editor(&path) {
        Ok(()) => {
            *state = CreatePushState::ReadyToValidate { skill, path };
        }
        Err(e) => {
            let prior = CreatePushState::ValidateErrors {
                skill,
                path,
                errors: vec![format!("editor: {}", e)],
            };
            *state = CreatePushState::Failed {
                state: Box::new(prior),
                message: format!("editor: {}", e),
            };
        }
    }
}

fn on_push(state: &mut CreatePushState, app: &mut App) {
    let (skill, remote, bump) = match state {
        CreatePushState::ReadyToPush {
            skill,
            remote,
            bump,
            ..
        } => (skill.clone(), remote.clone(), *bump),
        _ => return,
    };

    // Transition to Pushing state first so the spinner renders.
    *state = CreatePushState::Pushing {
        skill: skill.clone(),
        remote: remote.clone(),
        bump,
        started_at: Instant::now(),
        spinner: Spinner::default(),
    };

    // Defer the actual blocking push so the event loop renders the spinner
    // at least once before freezing.
    app.defer_blocking_action(BlockingAction::Push {
        skill,
        remote,
        bump: bump.as_bump_kind(),
    });
}

fn dismiss_failure(state: &mut CreatePushState, app: &mut App) {
    // Replace `state` with the boxed prior state.
    let placeholder = CreatePushState::Form(build_create_form_from_app(app));
    let old = std::mem::replace(state, placeholder);
    if let CreatePushState::Failed { state: prior, .. } = old {
        *state = *prior;
    }
}

// ---------------------------------------------------------------------------
// Render
// ---------------------------------------------------------------------------

/// Render the entire Create/Push screen for the current `state`.
pub fn render(frame: &mut Frame, app: &App, area: Rect, state: &CreatePushState) {
    match state {
        CreatePushState::Form(form) => form.render(area, frame.buffer_mut()),
        CreatePushState::ScaffoldRunning => {
            render_placeholder(frame, area, "Creating scaffold...");
        }
        CreatePushState::Editing { skill, .. } => {
            render_placeholder(frame, area, &format!("Editing {} in $EDITOR…", skill));
        }
        CreatePushState::ReadyToValidate { skill, .. } => {
            render_ready_to_validate(frame, area, skill);
        }
        CreatePushState::Validating { skill, .. } => {
            render_placeholder(frame, area, &format!("Validating {}…", skill));
        }
        CreatePushState::ValidateErrors { skill, errors, .. } => {
            render_validate_errors(frame, area, skill, errors);
        }
        CreatePushState::ReadyToPush {
            skill,
            remote,
            bump,
            ..
        } => {
            render_ready_to_push(frame, area, skill, remote.as_deref(), *bump);
        }
        CreatePushState::Pushing { skill, spinner, .. } => {
            render_pushing(frame, area, skill, spinner);
        }
        CreatePushState::Done(outcome) => {
            render_done(frame, area, outcome);
        }
        CreatePushState::Failed {
            message,
            state: inner,
        } => {
            render_inner_state(frame, app, area, inner);
            render_failure_banner(frame, area, message);
        }
    }
}

fn render_ready_to_validate(frame: &mut Frame, area: Rect, skill: &str) {
    let outer = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(" Ready to Validate ", theme::accent()));
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::raw("  Skill: "),
            Span::styled(skill, theme::accent()),
        ]),
        Line::from(""),
        Line::from("  Editing is complete. Run validation before pushing."),
        Line::from(""),
        Line::from(vec![Span::styled(
            "  [v] Validate   [e] Edit again   [c] Cancel",
            theme::dim(),
        )]),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_validate_errors(frame: &mut Frame, area: Rect, skill: &str, errors: &[String]) {
    let outer = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(" Validation Errors ", theme::accent()));
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let mut lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::raw("  Skill: "),
            Span::styled(skill, theme::accent()),
        ]),
        Line::from(""),
    ];
    for e in errors {
        lines.push(Line::from(vec![
            Span::styled("  ✗ ", Style::default().fg(Color::Red)),
            Span::raw(e.clone()),
        ]));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(vec![Span::styled(
        "  [e] Edit again   [c] Cancel",
        theme::dim(),
    )]));

    let items: Vec<ListItem> = lines.into_iter().map(ListItem::new).collect();
    frame.render_widget(
        List::new(items).block(Block::default().borders(Borders::NONE)),
        inner,
    );
}

fn render_ready_to_push(
    frame: &mut Frame,
    area: Rect,
    skill: &str,
    remote: Option<&str>,
    bump: BumpChoice,
) {
    let outer = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(" Ready to Push ", theme::accent()));
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let remote_display = remote.unwrap_or("(default)");

    let lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::raw("  Skill:  "),
            Span::styled(skill, theme::accent()),
        ]),
        Line::from(vec![
            Span::raw("  Remote: "),
            Span::styled(remote_display, theme::accent()),
        ]),
        Line::from(vec![
            Span::raw("  Bump:   "),
            Span::styled(bump.label(), theme::accent()),
            Span::styled(
                "  (b to cycle: patch / minor / major / as-written)",
                theme::dim(),
            ),
        ]),
        Line::from(""),
        Line::from(vec![Span::styled(
            "  [p] Push   [b] Cycle bump   [c] Cancel",
            theme::dim(),
        )]),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_pushing(frame: &mut Frame, area: Rect, skill: &str, spinner: &Spinner) {
    let outer = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(" Pushing ", theme::accent()));
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled(format!("  {} ", spinner.frame()), theme::accent()),
            Span::raw(format!("Pushing skill {}...", skill)),
        ]),
        Line::from(""),
        Line::from(vec![Span::styled(
            "  Please wait — this may take a few seconds.",
            theme::dim(),
        )]),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_done(frame: &mut Frame, area: Rect, outcome: &PushOutcome) {
    let outer = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(" Done ", theme::accent()));
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let header = Line::from(vec![Span::styled(
        format!(
            "  ✔ Pushed {} v{} → {}",
            outcome.skill, outcome.version, outcome.remote
        ),
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD),
    )]);

    let lines = if outcome.pr_url.is_empty() {
        // Direct-mode summary.
        let short_sha: String = outcome.commit_sha.chars().take(8).collect();
        vec![
            Line::from(""),
            header,
            Line::from(""),
            Line::from(vec![
                Span::raw("  Direct push to "),
                Span::styled(outcome.branch.clone(), theme::accent()),
                Span::raw(" at "),
                Span::styled(short_sha, theme::accent()),
            ]),
            Line::from(""),
            Line::from(vec![Span::styled(
                "  [b] back to dashboard",
                theme::dim(),
            )]),
        ]
    } else {
        // PR-mode summary.
        vec![
            Line::from(""),
            header,
            Line::from(""),
            Line::from(vec![
                Span::raw("  PR: "),
                Span::styled(outcome.pr_url.clone(), theme::accent()),
            ]),
            Line::from(""),
            Line::from(vec![Span::styled(
                "  [o] open in browser   [b] back to dashboard",
                theme::dim(),
            )]),
        ]
    };
    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_placeholder(frame: &mut Frame, area: Rect, msg: &str) {
    let outer = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(" Create / Push ", theme::accent()));
    let inner = outer.inner(area);
    frame.render_widget(outer, area);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw("  "),
            Span::raw(msg.to_string()),
        ])),
        inner,
    );
}

/// Render the underlying (non-Failed) state beneath a failure banner.
fn render_inner_state(frame: &mut Frame, app: &App, area: Rect, inner_state: &CreatePushState) {
    render(frame, app, area, inner_state);
}

fn render_failure_banner(frame: &mut Frame, area: Rect, message: &str) {
    let banner_height = 3_u16;
    let banner_area = Rect {
        x: area.x + 2,
        y: area.y + area.height.saturating_sub(banner_height + 2),
        width: area.width.saturating_sub(4),
        height: banner_height,
    };
    frame.render_widget(Clear, banner_area);
    let block = Block::default()
        .borders(Borders::ALL)
        .style(Style::default().bg(Color::Red).fg(Color::White))
        .title(" Error ");
    let inner = block.inner(banner_area);
    frame.render_widget(block, banner_area);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::raw(message),
            Span::styled("  (any key to dismiss)", Style::default().fg(Color::Yellow)),
        ])),
        inner,
    );
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn make_app() -> App {
        App::new(
            quay_core::Config::default(),
            quay_core::Lockfile::default(),
            std::path::PathBuf::from("/tmp"),
            None,
        )
    }

    #[test]
    fn bump_choice_cycles() {
        let b = BumpChoice::Patch;
        let b = b.next();
        assert!(matches!(b, BumpChoice::Minor));
        let b = b.next();
        assert!(matches!(b, BumpChoice::Major));
        let b = b.next();
        assert!(matches!(b, BumpChoice::AsWritten));
        let b = b.next();
        assert!(matches!(b, BumpChoice::Patch));
    }

    #[test]
    fn spinner_advances_in_pushing_state() {
        let mut state = CreatePushState::Pushing {
            skill: "x".into(),
            remote: None,
            bump: BumpChoice::Patch,
            started_at: Instant::now(),
            spinner: Spinner::default(),
        };
        let frame0 = if let CreatePushState::Pushing { spinner, .. } = &state {
            spinner.frame()
        } else {
            unreachable!()
        };
        state.tick();
        let frame1 = if let CreatePushState::Pushing { spinner, .. } = &state {
            spinner.frame()
        } else {
            unreachable!()
        };
        assert_ne!(frame0, frame1);
    }

    #[test]
    fn failed_state_dismisses_on_any_key() {
        let mut app = make_app();
        app.create_push = CreatePushState::Failed {
            state: Box::new(CreatePushState::Form(build_create_form(&[]))),
            message: "oops".into(),
        };
        handle_key(&mut app, KeyCode::Enter);
        assert!(matches!(app.create_push, CreatePushState::Form(_)));
    }

    #[test]
    fn done_o_key_opens_browser_and_sets_status() {
        let mut app = make_app();
        app.create_push = CreatePushState::Done(PushOutcome {
            skill: "s".into(),
            remote: "r".into(),
            branch: "b".into(),
            version: "0.1.0".into(),
            pr_url: "https://example.com/pr/1".into(),
            pr_auto_created: true,
            commit_sha: "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef".into(),
        });
        handle_key(&mut app, KeyCode::Char('o'));
        assert!(app.status_message.is_some());
        let msg = app.status_message.as_ref().unwrap();
        assert!(
            msg.starts_with("opened: "),
            "expected status to start with 'opened: ', got: {msg}"
        );
        assert!(
            msg.contains("https://example.com/pr/1"),
            "expected status to contain the URL, got: {msg}"
        );
    }

    #[test]
    fn ready_to_push_bump_cycles_via_b_key() {
        let mut app = make_app();
        app.create_push = CreatePushState::ReadyToPush {
            skill: "x".into(),
            path: PathBuf::from("/tmp/x"),
            remote: None,
            bump: BumpChoice::Patch,
        };
        handle_key(&mut app, KeyCode::Char('b'));
        assert!(
            matches!(
                app.create_push,
                CreatePushState::ReadyToPush {
                    bump: BumpChoice::Minor,
                    ..
                }
            ),
            "expected Minor bump"
        );
    }

    // -- Paste handler tests --

    #[test]
    fn paste_inserts_into_focused_name_field() {
        let mut state = CreatePushState::Form(build_create_form(&[]));
        handle_paste(&mut state, "my-skill");
        if let CreatePushState::Form(form) = &state {
            let json = form.to_json();
            assert_eq!(json["name"].as_str().unwrap_or(""), "my-skill");
        } else {
            panic!("wrong state");
        }
    }

    #[test]
    fn paste_inserts_into_description_when_focused() {
        let mut state = CreatePushState::Form(build_create_form(&[]));
        // Tab once: name → description.
        if let CreatePushState::Form(form) = &mut state {
            let tab = KeyEvent {
                code: KeyCode::Tab,
                modifiers: KeyModifiers::NONE,
                kind: KeyEventKind::Press,
                state: KeyEventState::NONE,
            };
            form.handle_input(tab);
        }
        handle_paste(&mut state, "A short description");
        if let CreatePushState::Form(form) = &state {
            let json = form.to_json();
            assert_eq!(
                json["description"].as_str().unwrap_or(""),
                "A short description"
            );
        } else {
            panic!("wrong state");
        }
    }

    #[test]
    fn paste_noop_when_not_in_form_state() {
        let mut state = CreatePushState::ReadyToValidate {
            skill: "s".into(),
            path: std::path::PathBuf::from("/tmp/s"),
        };
        handle_paste(&mut state, "some text");
        assert!(matches!(state, CreatePushState::ReadyToValidate { .. }));
    }

    #[test]
    fn form_render_does_not_panic() {
        let mut term = Terminal::new(TestBackend::new(80, 24)).unwrap();
        let app = make_app();
        let state = CreatePushState::Form(build_create_form(
            &["skills-hub", "platform"]
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>(),
        ));
        term.draw(|f| render(f, &app, f.area(), &state)).unwrap();
        let buf = term.backend().buffer().clone();
        // Form renders without panic; lib controls content.
        assert!(buf.area().width > 0);
    }

    // Tests involving scaffold / validate / push transitions that need EDITOR=true
    // are marked #[ignore] because std::env::set_var is not safe to call from
    // multiple concurrent test threads (UB under POSIX).
    // Run with: cargo test -p quay-cli -- --ignored

    #[test]
    #[ignore]
    fn on_save_transitions_to_ready_to_validate_with_true_editor() {
        let dir = assert_fs::TempDir::new().unwrap();
        let cfg = quay_core::Config::default();
        let lock = quay_core::Lockfile::default();
        let mut app = App::new(cfg, lock, dir.path().to_path_buf(), None);

        // Build a form with "my-skill" filled in the name field.
        let mut form = build_create_form(&[]);
        let chars: Vec<KeyEvent> = "my-skill"
            .chars()
            .map(|c| KeyEvent {
                code: KeyCode::Char(c),
                modifiers: KeyModifiers::NONE,
                kind: KeyEventKind::Press,
                state: KeyEventState::NONE,
            })
            .collect();
        for ev in chars {
            form.handle_input(ev);
        }
        // Tab to description, type something.
        let tab = KeyEvent {
            code: KeyCode::Tab,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        };
        form.handle_input(tab);
        for ev in "a skill".chars().map(|c| KeyEvent {
            code: KeyCode::Char(c),
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }) {
            form.handle_input(ev);
        }
        let mut state = CreatePushState::Form(form);

        // SAFETY: single-threaded when run with -- --ignored
        unsafe {
            std::env::set_var("EDITOR", "true");
        }
        // Tab to Submit and press Enter.
        // We submit name "my-skill" directly via on_save.
        on_save(&mut state, &mut app, "my-skill");
        unsafe {
            std::env::remove_var("EDITOR");
        }
        assert!(
            matches!(state, CreatePushState::ReadyToValidate { .. }),
            "expected ReadyToValidate, got: {:?}",
            state
        );
    }

    #[test]
    fn on_validate_transitions_to_ready_to_push_when_valid() {
        let dir = assert_fs::TempDir::new().unwrap();
        let skill_dir = dir.path().join(".agents/skills/csv-parse");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: csv-parse\ndescription: Parse CSV.\nversion: 1.0.0\n---\nbody\n",
        )
        .unwrap();

        let mut cfg = quay_core::Config::default();
        cfg.remotes.insert(
            "hub".into(),
            quay_core::RemoteConfig {
                url: "https://example.com/hub.git".into(),
                default: true,
                provider: None,
                push_mode: quay_core::PushMode::default(),
            },
        );
        let lock = quay_core::Lockfile::default();
        let mut app = App::new(cfg, lock, dir.path().to_path_buf(), None);

        let mut state = CreatePushState::ReadyToValidate {
            skill: "csv-parse".into(),
            path: skill_dir.join("SKILL.md"),
        };
        on_validate(&mut state, &mut app);
        assert!(
            matches!(state, CreatePushState::ReadyToPush { .. }),
            "expected ReadyToPush, got: {:?}",
            state
        );
    }

    #[test]
    fn on_validate_transitions_to_errors_when_invalid() {
        let dir = assert_fs::TempDir::new().unwrap();
        let skill_dir = dir.path().join(".agents/skills/bad-skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: bad-skill\n---\nbody\n",
        )
        .unwrap();

        let cfg = quay_core::Config::default();
        let lock = quay_core::Lockfile::default();
        let mut app = App::new(cfg, lock, dir.path().to_path_buf(), None);

        let mut state = CreatePushState::ReadyToValidate {
            skill: "bad-skill".into(),
            path: skill_dir.join("SKILL.md"),
        };
        on_validate(&mut state, &mut app);
        assert!(
            matches!(state, CreatePushState::ValidateErrors { .. }),
            "expected ValidateErrors, got: {:?}",
            state
        );
    }

    #[test]
    fn on_push_transitions_to_pushing_state() {
        let cfg = quay_core::Config::default();
        let lock = quay_core::Lockfile::default();
        let mut app = App::new(cfg, lock, std::path::PathBuf::from("/tmp"), None);
        let mut state = CreatePushState::ReadyToPush {
            skill: "my-skill".into(),
            path: PathBuf::from("/tmp/my-skill/SKILL.md"),
            remote: Some("hub".into()),
            bump: BumpChoice::Patch,
        };
        on_push(&mut state, &mut app);
        assert!(
            matches!(state, CreatePushState::Pushing { .. }),
            "expected Pushing, got: {:?}",
            state
        );
        assert!(
            app.next_blocking.is_some(),
            "expected a deferred BlockingAction"
        );
    }
}
