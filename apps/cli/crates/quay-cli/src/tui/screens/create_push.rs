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
use crossterm::event::KeyCode;
use quay_core::BumpKind;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph};
use ratatui::Frame;
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

/// Focus position within the Create form.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FormFocus {
    #[default]
    Name,
    Description,
    Tags,
    Remote,
    SaveButton,
    CancelButton,
}

impl FormFocus {
    fn next(self) -> FormFocus {
        match self {
            FormFocus::Name => FormFocus::Description,
            FormFocus::Description => FormFocus::Tags,
            FormFocus::Tags => FormFocus::Remote,
            FormFocus::Remote => FormFocus::SaveButton,
            FormFocus::SaveButton => FormFocus::CancelButton,
            FormFocus::CancelButton => FormFocus::Name,
        }
    }

    fn prev(self) -> FormFocus {
        match self {
            FormFocus::Name => FormFocus::CancelButton,
            FormFocus::Description => FormFocus::Name,
            FormFocus::Tags => FormFocus::Description,
            FormFocus::Remote => FormFocus::Tags,
            FormFocus::SaveButton => FormFocus::Remote,
            FormFocus::CancelButton => FormFocus::SaveButton,
        }
    }
}

/// Inline form fields for the Create Skill form.
#[derive(Debug, Clone)]
pub struct FormFields {
    pub name: String,
    pub description: String,
    /// Comma-separated tags entered as a single string.
    pub tags: String,
    /// Available remote names from the active config.
    pub remotes: Vec<String>,
    /// Index of the currently selected remote in `remotes`.
    pub remote_idx: usize,
    pub focus: FormFocus,
}

impl FormFields {
    /// Create a form pre-populated with the given remote names.
    pub fn with_remotes(remotes: &[&str]) -> Self {
        Self {
            name: String::new(),
            description: String::new(),
            tags: String::new(),
            remotes: remotes.iter().map(|s| s.to_string()).collect(),
            remote_idx: 0,
            focus: FormFocus::Name,
        }
    }

    /// Create a form from the app's current config remotes.
    pub fn from_app(app: &App) -> Self {
        Self::from_config_remotes(&app.cfg)
    }

    /// Create a form pre-populated from a [`quay_core::Config`]'s remotes.
    pub fn from_config_remotes(cfg: &quay_core::Config) -> Self {
        let remotes: Vec<String> = cfg.remotes.keys().cloned().collect();
        let remote_idx = cfg
            .default_remote()
            .and_then(|(name, _)| remotes.iter().position(|r| r == name))
            .unwrap_or(0);
        Self {
            name: String::new(),
            description: String::new(),
            tags: String::new(),
            remotes,
            remote_idx,
            focus: FormFocus::Name,
        }
    }

    /// Currently selected remote name, if any.
    pub fn selected_remote(&self) -> Option<&str> {
        self.remotes.get(self.remote_idx).map(|s| s.as_str())
    }

    /// Advance focus forward (Tab).
    pub fn tab_forward(&mut self) {
        self.focus = self.focus.next();
    }

    /// Advance focus backward (BackTab / Shift-Tab).
    pub fn tab_back(&mut self) {
        self.focus = self.focus.prev();
    }

    /// Handle a character typed into the currently focused text field.
    fn push_char(&mut self, c: char) {
        match self.focus {
            FormFocus::Name => self.name.push(c),
            FormFocus::Description => self.description.push(c),
            FormFocus::Tags => self.tags.push(c),
            FormFocus::Remote => {
                // Cycle remote selection on left/right — handled separately.
                // Ignore typed characters in the remote field.
            }
            FormFocus::SaveButton | FormFocus::CancelButton => {}
        }
    }

    fn pop_char(&mut self) {
        match self.focus {
            FormFocus::Name => {
                self.name.pop();
            }
            FormFocus::Description => {
                self.description.pop();
            }
            FormFocus::Tags => {
                self.tags.pop();
            }
            _ => {}
        }
    }

    fn cycle_remote_next(&mut self) {
        if !self.remotes.is_empty() {
            self.remote_idx = (self.remote_idx + 1) % self.remotes.len();
        }
    }

    fn cycle_remote_prev(&mut self) {
        if !self.remotes.is_empty() {
            if self.remote_idx == 0 {
                self.remote_idx = self.remotes.len() - 1;
            } else {
                self.remote_idx -= 1;
            }
        }
    }
}

/// Full state machine for Screen 5.
#[derive(Debug)]
pub enum CreatePushState {
    /// Collecting frontmatter from the user.
    Form(FormFields),
    /// Scaffold is running (fast disk write — this state is mostly a visual
    /// marker; we transition through it synchronously before entering `Editing`).
    ScaffoldRunning(FormFields),
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

impl CreatePushState {
    /// Advance the spinner if we are in the `Pushing` state.
    pub fn tick(&mut self) {
        if let CreatePushState::Pushing { spinner, .. } = self {
            spinner.advance();
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
    let placeholder = CreatePushState::Form(FormFields::with_remotes(&[]));
    let mut state = std::mem::replace(&mut app.create_push, placeholder);

    let action = handle_key_inner(&mut state, app, code);

    // Move state back.
    app.create_push = state;
    action
}

fn handle_key_inner(state: &mut CreatePushState, app: &mut App, code: KeyCode) -> ScreenAction {
    // Dispatch by taking a snapshot of the discriminant — avoids holding a
    // borrow on `state` while we subsequently move into `state`.
    let discriminant = state_discriminant(state);
    match discriminant {
        StateKind::Form => handle_form(state, app, code),
        StateKind::ReadyToValidate => handle_ready_to_validate(state, app, code),
        StateKind::ValidateErrors => handle_validate_errors(state, app, code),
        StateKind::ReadyToPush => handle_ready_to_push(state, app, code),
        StateKind::Done => handle_done(state, app, code),
        StateKind::Failed => {
            dismiss_failure(state);
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
    let fields_focus = match state {
        CreatePushState::Form(f) => f.focus,
        _ => return ScreenAction::Stay,
    };

    match code {
        KeyCode::Esc => {
            return ScreenAction::SwitchTo(crate::tui::app::Screen::Dashboard);
        }
        KeyCode::Tab => {
            if let CreatePushState::Form(f) = state {
                f.tab_forward();
            }
        }
        KeyCode::BackTab => {
            if let CreatePushState::Form(f) = state {
                f.tab_back();
            }
        }
        KeyCode::Left => {
            if fields_focus == FormFocus::Remote {
                if let CreatePushState::Form(f) = state {
                    f.cycle_remote_prev();
                }
            }
        }
        KeyCode::Right => {
            if fields_focus == FormFocus::Remote {
                if let CreatePushState::Form(f) = state {
                    f.cycle_remote_next();
                }
            }
        }
        KeyCode::Backspace => {
            if let CreatePushState::Form(f) = state {
                f.pop_char();
            }
        }
        KeyCode::Char(c) => match fields_focus {
            FormFocus::SaveButton | FormFocus::CancelButton => {}
            _ => {
                if let CreatePushState::Form(f) = state {
                    f.push_char(c);
                }
            }
        },
        KeyCode::Enter => match fields_focus {
            FormFocus::CancelButton => {
                return ScreenAction::SwitchTo(crate::tui::app::Screen::Dashboard);
            }
            _ => {
                on_save(state, app);
                return ScreenAction::Stay;
            }
        },
        _ => {}
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

fn handle_done(state: &mut CreatePushState, app: &mut App, code: KeyCode) -> ScreenAction {
    match code {
        KeyCode::Char('o') => {
            // Stub: URL-open not available in this plan.
            if let CreatePushState::Done(outcome) = state {
                app.set_status(format!("PR open: {}", outcome.pr_url));
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

fn on_save(state: &mut CreatePushState, app: &mut App) {
    // Extract fields by cloning out of the Form variant.
    let fields = match state {
        CreatePushState::Form(f) => f.clone(),
        _ => return,
    };

    let name = fields.name.trim().to_string();
    if name.is_empty() {
        app.set_status("skill name is required");
        return;
    }

    match commands::create::scaffold(
        &name,
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
                        state: Box::new(CreatePushState::Form(fields)),
                        message: format!("editor: {}", e),
                    };
                }
            }
        }
        Err(e) => {
            *state = CreatePushState::Failed {
                state: Box::new(CreatePushState::Form(fields)),
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

    match commands::validate::validate_skill(&skill, &app.project_root) {
        Ok(outcome) if outcome.errors.is_empty() => {
            // Find the default remote from the app config.
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
                errors: outcome.errors,
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

fn dismiss_failure(state: &mut CreatePushState) {
    // Replace `state` with the boxed prior state.
    // We need to take ownership using a placeholder.
    let placeholder = CreatePushState::Form(FormFields::with_remotes(&[]));
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
        CreatePushState::Form(fields) => render_form(frame, area, fields),
        CreatePushState::ScaffoldRunning(_) => {
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

fn render_form(frame: &mut Frame, area: Rect, fields: &FormFields) {
    let outer = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(" Create Skill ", theme::accent()));
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // header hint
            Constraint::Length(2), // Name field
            Constraint::Length(2), // Description
            Constraint::Length(2), // Tags
            Constraint::Length(2), // Remote
            Constraint::Length(1), // spacer
            Constraint::Length(1), // buttons
            Constraint::Length(1), // footer
        ])
        .split(inner);

    frame.render_widget(
        Paragraph::new(Span::styled(
            "Fill in the skill metadata, then press Enter to open $EDITOR for the body.",
            theme::dim(),
        )),
        rows[0],
    );

    render_field(
        frame,
        rows[1],
        "Name",
        &fields.name,
        fields.focus == FormFocus::Name,
    );
    render_field(
        frame,
        rows[2],
        "Description",
        &fields.description,
        fields.focus == FormFocus::Description,
    );
    render_field(
        frame,
        rows[3],
        "Tags (comma-separated)",
        &fields.tags,
        fields.focus == FormFocus::Tags,
    );

    let remote_label = if fields.remotes.is_empty() {
        "(no remotes configured)".to_string()
    } else {
        fields
            .remotes
            .get(fields.remote_idx)
            .cloned()
            .unwrap_or_default()
    };
    render_field(
        frame,
        rows[4],
        "Remote  ← →",
        &remote_label,
        fields.focus == FormFocus::Remote,
    );

    let save_style = if fields.focus == FormFocus::SaveButton {
        theme::selected()
    } else {
        Style::default()
    };
    let cancel_style = if fields.focus == FormFocus::CancelButton {
        theme::selected()
    } else {
        Style::default()
    };
    let buttons = Line::from(vec![
        Span::styled(" [Enter] Save & open editor ", save_style),
        Span::raw("   "),
        Span::styled(" [Esc/Cancel] ", cancel_style),
    ]);
    frame.render_widget(Paragraph::new(buttons), rows[6]);

    frame.render_widget(
        Paragraph::new(Span::styled(
            "Tab / BackTab — cycle fields   Esc — cancel",
            theme::dim(),
        )),
        rows[7],
    );
}

fn render_field(frame: &mut Frame, area: Rect, label: &str, value: &str, focused: bool) {
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
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(cursor, label_style),
            Span::styled(label, label_style),
        ])),
        rows[0],
    );

    let value_style = if focused {
        Style::default().add_modifier(Modifier::UNDERLINED)
    } else {
        Style::default()
    };
    let display = format!("  {}", value);
    frame.render_widget(Paragraph::new(Span::styled(display, value_style)), rows[1]);
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

    let lines = vec![
        Line::from(""),
        Line::from(vec![Span::styled(
            format!(
                "  ✔ Pushed {} v{} → {}",
                outcome.skill, outcome.version, outcome.remote
            ),
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        )]),
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
    ];
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

    fn buf_contains(buf: &ratatui::buffer::Buffer, needle: &str) -> bool {
        let content: String = buf.content().iter().map(|c| c.symbol()).collect();
        content.contains(needle)
    }

    #[test]
    fn form_renders_all_fields() {
        let mut term = Terminal::new(TestBackend::new(80, 24)).unwrap();
        let cfg = quay_core::Config::default();
        let lock = quay_core::Lockfile::default();
        let app = App::new(cfg, lock, std::path::PathBuf::from("/tmp"), None);
        let state = CreatePushState::Form(FormFields::with_remotes(&["skills-hub", "platform"]));
        term.draw(|f| render(f, &app, f.area(), &state)).unwrap();
        let buf = term.backend().buffer().clone();
        assert!(buf_contains(&buf, "Name"), "buffer missing 'Name'");
        assert!(
            buf_contains(&buf, "Description"),
            "buffer missing 'Description'"
        );
        assert!(buf_contains(&buf, "Tags"), "buffer missing 'Tags'");
        assert!(
            buf_contains(&buf, "skills-hub"),
            "buffer missing remote 'skills-hub'"
        );
    }

    #[test]
    fn tab_advances_focus_through_all_fields() {
        let mut fields = FormFields::with_remotes(&["a"]);
        assert!(matches!(fields.focus, FormFocus::Name));
        fields.tab_forward();
        assert!(matches!(fields.focus, FormFocus::Description));
        fields.tab_forward();
        assert!(matches!(fields.focus, FormFocus::Tags));
        fields.tab_forward();
        assert!(matches!(fields.focus, FormFocus::Remote));
        fields.tab_forward();
        assert!(matches!(fields.focus, FormFocus::SaveButton));
        fields.tab_forward();
        assert!(matches!(fields.focus, FormFocus::CancelButton));
        fields.tab_forward();
        // Wraps back to Name.
        assert!(matches!(fields.focus, FormFocus::Name));
    }

    #[test]
    fn backtab_reverses_focus() {
        let mut fields = FormFields::with_remotes(&["a"]);
        fields.tab_back();
        assert!(matches!(fields.focus, FormFocus::CancelButton));
    }

    #[test]
    fn char_input_appends_to_name_field() {
        let mut fields = FormFields::with_remotes(&["a"]);
        fields.push_char('h');
        fields.push_char('i');
        assert_eq!(fields.name, "hi");
    }

    #[test]
    fn backspace_removes_last_char() {
        let mut fields = FormFields::with_remotes(&["a"]);
        fields.push_char('h');
        fields.push_char('i');
        fields.pop_char();
        assert_eq!(fields.name, "h");
    }

    #[test]
    fn remote_cycles_on_left_right() {
        let mut fields = FormFields::with_remotes(&["a", "b", "c"]);
        fields.focus = FormFocus::Remote;
        assert_eq!(fields.selected_remote(), Some("a"));
        fields.cycle_remote_next();
        assert_eq!(fields.selected_remote(), Some("b"));
        fields.cycle_remote_next();
        assert_eq!(fields.selected_remote(), Some("c"));
        fields.cycle_remote_next();
        // Wraps.
        assert_eq!(fields.selected_remote(), Some("a"));
        fields.cycle_remote_prev();
        assert_eq!(fields.selected_remote(), Some("c"));
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
        let cfg = quay_core::Config::default();
        let lock = quay_core::Lockfile::default();
        let mut app = App::new(cfg, lock, std::path::PathBuf::from("/tmp"), None);
        app.create_push = CreatePushState::Failed {
            state: Box::new(CreatePushState::Form(FormFields::with_remotes(&[]))),
            message: "oops".into(),
        };
        handle_key(&mut app, KeyCode::Enter);
        assert!(matches!(app.create_push, CreatePushState::Form(_)));
    }

    #[test]
    fn done_o_key_sets_status_message() {
        let cfg = quay_core::Config::default();
        let lock = quay_core::Lockfile::default();
        let mut app = App::new(cfg, lock, std::path::PathBuf::from("/tmp"), None);
        app.create_push = CreatePushState::Done(PushOutcome {
            skill: "s".into(),
            remote: "r".into(),
            branch: "b".into(),
            version: "0.1.0".into(),
            pr_url: "https://example.com/pr/1".into(),
            pr_auto_created: true,
        });
        handle_key(&mut app, KeyCode::Char('o'));
        assert!(app.status_message.is_some());
        assert!(app
            .status_message
            .as_ref()
            .unwrap()
            .contains("https://example.com/pr/1"));
    }

    #[test]
    fn ready_to_push_bump_cycles_via_b_key() {
        let cfg = quay_core::Config::default();
        let lock = quay_core::Lockfile::default();
        let mut app = App::new(cfg, lock, std::path::PathBuf::from("/tmp"), None);
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
        let mut fields = FormFields::with_remotes(&[]);
        fields.name = "my-skill".into();
        let mut state = CreatePushState::Form(fields);

        // SAFETY: single-threaded when run with -- --ignored
        unsafe {
            std::env::set_var("EDITOR", "true");
        }
        on_save(&mut state, &mut app);
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
        // Minimal frontmatter missing required fields (description, version).
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
        // The blocking action was deferred.
        assert!(
            app.next_blocking.is_some(),
            "expected a deferred BlockingAction"
        );
    }
}
