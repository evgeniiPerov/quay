//! Screen 5 — Create / Push.
//!
//! State machine:
//! `Form` → `ScaffoldRunning` → `Editing` → `ReadyToValidate` → `Validating`
//!   → `ValidateErrors` | `ReadyToPush` → `Pushing` → `Done`
//!
//! `PushModal` is a lightweight push-existing-skill form launched from Local
//! `[u]`/`[U]`: it collects Tags, Bump, and Target remote, then transitions to
//! `ReadyToPush` → `Pushing` → `Done` exactly like the create flow.
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
use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::Instant;

use crate::commands::push::PushOutcome;

/// Per-skill outcome recorded during a bulk push.
#[derive(Debug, Clone)]
pub struct BulkResult {
    /// Skill name.
    pub skill: String,
    /// `Ok` with push outcome, or `Err` with error message.
    pub outcome: Result<PushOutcome, String>,
}

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
// Form builders
// ---------------------------------------------------------------------------

/// Build the Create Skill frontmatter form (used by the Dashboard `[u]` shortcut
/// and the global `[c]` hotkey in older plans).
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

/// Build a fresh create form pre-populated from the app's current config remotes.
pub fn build_create_form_from_app(app: &App) -> Form {
    let remotes: Vec<String> = app.cfg.remotes.keys().cloned().collect();
    build_create_form(&remotes)
}

/// Build the Push Existing Skill form launched from Local `[u]`/`[U]`.
///
/// Fields (in order):
/// - Tags (text, comma-separated, pre-filled from skill frontmatter)
/// - Bump (select: patch / minor / major / as-written)
/// - Target remote (select over configured remotes; omitted when empty)
///
/// The form title is `Push <skill_name>` with the actual name embedded.
pub fn build_push_form(skill_name: &str, tags_initial: &str, remotes: &[String]) -> Form {
    let title = format!(" Push {} ", skill_name);
    let mut b = Form::builder()
        .title(title)
        .style(crate::tui::form_theme::dark())
        .text("tags", "Tags (comma-separated)")
        .initial_value(tags_initial)
        .done()
        .select("bump", "Bump")
        .options(vec![
            ("patch", "Patch  (0.1.0 \u{2192} 0.1.1)"),
            ("minor", "Minor  (0.1.0 \u{2192} 0.2.0)"),
            ("major", "Major  (0.1.0 \u{2192} 1.0.0)"),
            ("as-written", "As-written (no version change)"),
        ])
        .initial_value("patch")
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

/// Build a push form pre-populated from the app's config remotes.
pub fn build_push_form_from_app(skill_name: &str, tags_initial: &str, app: &App) -> Form {
    let remotes: Vec<String> = app.cfg.remotes.keys().cloned().collect();
    build_push_form(skill_name, tags_initial, &remotes)
}

/// Build a push-existing form launched from Local `[U]`.
///
/// Like [`build_push_form`] but accepts an explicit remote list and a
/// `default_remote` to pre-select.  When `default_remote` is `None` the
/// first remote in `remotes` is pre-selected (same behaviour as
/// [`build_push_form`]).
pub fn build_push_existing_form(
    skill_name: &str,
    tags: &[String],
    remotes: &[String],
    default_remote: Option<&str>,
) -> Box<Form> {
    let tags_initial = tags.join(", ");
    let title = format!(" Push {} ", skill_name);
    let mut b = Form::builder()
        .title(title)
        .style(crate::tui::form_theme::dark())
        .text("tags", "Tags (comma-separated)")
        .initial_value(tags_initial.as_str())
        .done()
        .select("bump", "Bump")
        .options(vec![
            ("patch", "Patch  (0.1.0 \u{2192} 0.1.1)"),
            ("minor", "Minor  (0.1.0 \u{2192} 0.2.0)"),
            ("major", "Major  (0.1.0 \u{2192} 1.0.0)"),
            ("as-written", "As-written (no version change)"),
        ])
        .initial_value("patch")
        .done();
    if !remotes.is_empty() {
        let initial = default_remote.unwrap_or(remotes[0].as_str());
        let options: Vec<(&str, &str)> = remotes.iter().map(|s| (s.as_str(), s.as_str())).collect();
        b = b
            .select("remote", "Target remote")
            .options(options)
            .initial_value(initial)
            .done();
    }
    Box::new(b.build())
}

// ---------------------------------------------------------------------------
// State machine
// ---------------------------------------------------------------------------

/// Full state machine for Screen 5.
pub enum CreatePushState {
    /// Collecting frontmatter from the user via `ratatui-form` (create flow).
    Form(ratatui_form::Form),
    /// Push-existing-skill modal: collects Tags, Bump, Target remote.
    ///
    /// Launched from Local `[u]`/`[U]`. Esc returns to Local; submit transitions
    /// to `ReadyToPush` (which re-uses the shared push pipeline).
    PushModal {
        /// Display name of the skill being pushed.
        skill_name: String,
        /// Absolute path to the skill's canonical `SKILL.md`.
        skill_path: PathBuf,
        /// The ratatui-form collecting Tags + Bump + Target remote.
        form: Box<ratatui_form::Form>,
    },
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
    /// Push-existing form launched from Local `[U]` (simplified variant used
    /// when skill path is resolved upstream in Local).
    ///
    /// `skill` is the skill name; `form` is the pre-built form box from
    /// [`build_push_existing_form`].  Esc returns to Local; submit transitions
    /// to `ReadyToPush`.
    PushExistingForm {
        /// Display name of the skill being pushed.
        skill: String,
        /// The ratatui-form collecting Tags + Bump + Target remote.
        form: Box<ratatui_form::Form>,
    },
    /// An operation failed; `message` is shown as a banner.  The boxed `state`
    /// is the prior stable state we will return to on acknowledgement.
    Failed {
        state: Box<CreatePushState>,
        message: String,
    },
    /// Bulk push form: collects bump + remote for N picked skills.
    ///
    /// On submit, transitions to `BulkPushing` via `bulk_form_push`.
    BulkPushForm {
        /// Names of the skills to push (in pick order).
        skill_names: Vec<String>,
        /// The ratatui-form collecting Tags + Bump + Target remote.
        form: Box<ratatui_form::Form>,
    },
    /// Bulk push in progress: one skill at a time, driven sequentially.
    BulkPushing {
        /// Skill names still waiting to be pushed.
        remaining: VecDeque<String>,
        /// Total number of skills in the batch (for progress display).
        total: usize,
        /// Skill currently being pushed.
        current: String,
        /// Bump level applied to every skill in the batch.
        bump: BumpChoice,
        /// Optional remote override (None = default remote).
        remote: Option<String>,
        /// Accumulated per-skill outcomes.
        results: Vec<BulkResult>,
        /// Spinner widget.
        spinner: Spinner,
        /// When the batch started (for elapsed-time display).
        started_at: Instant,
    },
    /// All bulk pushes finished — show per-skill outcome list.
    BulkDone(Vec<BulkResult>),
}

impl std::fmt::Debug for CreatePushState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CreatePushState::Form(_) => write!(f, "Form(...)"),
            CreatePushState::PushModal { skill_name, .. } => {
                write!(f, "PushModal({})", skill_name)
            }
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
            CreatePushState::PushExistingForm { skill, .. } => {
                write!(f, "PushExistingForm({})", skill)
            }
            CreatePushState::Done(o) => write!(f, "Done({:?})", o),
            CreatePushState::Failed { message, .. } => {
                f.debug_struct("Failed").field("message", message).finish()
            }
            CreatePushState::BulkPushForm { skill_names, .. } => {
                write!(f, "BulkPushForm({} skills)", skill_names.len())
            }
            CreatePushState::BulkPushing {
                current,
                total,
                results,
                ..
            } => f
                .debug_struct("BulkPushing")
                .field("current", current)
                .field("total", total)
                .field("done", &results.len())
                .finish(),
            CreatePushState::BulkDone(results) => {
                write!(f, "BulkDone({} results)", results.len())
            }
        }
    }
}

impl CreatePushState {
    /// Advance the spinner if we are in the `Pushing` or `BulkPushing` state.
    pub fn tick(&mut self) {
        match self {
            CreatePushState::Pushing { spinner, .. }
            | CreatePushState::BulkPushing { spinner, .. } => {
                spinner.advance();
            }
            _ => {}
        }
    }

    /// Returns `true` when this state is a `PushModal` (i.e. came from Local `[u]`).
    ///
    /// Used by the event loop so it does not clobber a pre-populated push form
    /// when switching to `Screen::CreatePush`.
    pub fn is_push_modal(&self) -> bool {
        matches!(
            self,
            CreatePushState::PushModal { .. }
                | CreatePushState::PushExistingForm { .. }
                | CreatePushState::BulkPushForm { .. }
                | CreatePushState::BulkPushing { .. }
                | CreatePushState::BulkDone(_)
        )
    }
}

// ---------------------------------------------------------------------------
// Paste handler
// ---------------------------------------------------------------------------

/// Insert a pasted string into the currently focused text field of the form.
///
/// Only the `Form` and `PushModal` states accept paste; all other states in
/// the state machine silently drop the paste.
pub fn handle_paste(state: &mut CreatePushState, s: &str) {
    let events = crate::tui::paste_to_key_events(s);
    match state {
        CreatePushState::Form(form) => {
            for ev in events {
                form.handle_input(ev);
            }
        }
        CreatePushState::PushModal { form, .. } => {
            for ev in events {
                form.handle_input(ev);
            }
        }
        CreatePushState::PushExistingForm { form, .. } => {
            for ev in events {
                form.handle_input(ev);
            }
        }
        CreatePushState::BulkPushForm { form, .. } => {
            for ev in events {
                form.handle_input(ev);
            }
        }
        _ => {}
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
        StateKind::PushModal => handle_push_modal(state, app, code),
        StateKind::PushExistingForm => handle_push_existing_form(state, app, code),
        StateKind::BulkPushForm => handle_bulk_push_form(state, app, code),
        StateKind::ReadyToValidate => handle_ready_to_validate(state, app, code),
        StateKind::ValidateErrors => handle_validate_errors(state, app, code),
        StateKind::ReadyToPush => handle_ready_to_push(state, app, code),
        StateKind::Done => handle_done(state, app, code),
        StateKind::BulkDone => handle_bulk_done(state, app, code),
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
    PushModal,
    PushExistingForm,
    BulkPushForm,
    ReadyToValidate,
    ValidateErrors,
    ReadyToPush,
    Done,
    Failed,
    BulkDone,
    Other,
}

fn state_discriminant(state: &CreatePushState) -> StateKind {
    match state {
        CreatePushState::Form(_) => StateKind::Form,
        CreatePushState::PushModal { .. } => StateKind::PushModal,
        CreatePushState::PushExistingForm { .. } => StateKind::PushExistingForm,
        CreatePushState::BulkPushForm { .. } => StateKind::BulkPushForm,
        CreatePushState::ReadyToValidate { .. } => StateKind::ReadyToValidate,
        CreatePushState::ValidateErrors { .. } => StateKind::ValidateErrors,
        CreatePushState::ReadyToPush { .. } => StateKind::ReadyToPush,
        CreatePushState::Done(_) => StateKind::Done,
        CreatePushState::Failed { .. } => StateKind::Failed,
        CreatePushState::BulkDone(_) => StateKind::BulkDone,
        _ => StateKind::Other,
    }
}

fn handle_form(state: &mut CreatePushState, app: &mut App, code: KeyCode) -> ScreenAction {
    // Intercept Esc before delegating to the form — Esc cancels to Dashboard.
    if code == KeyCode::Esc {
        return ScreenAction::SwitchTo(crate::tui::app::Screen::Dashboard);
    }

    // Translate BackTab -> Tab + SHIFT (ratatui-form checks modifiers).
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

/// Handle keys for the push-existing-skill modal form.
///
/// Esc returns to Local without changes. Submit writes updated tags to disk
/// and transitions to `ReadyToPush`.
fn handle_push_modal(state: &mut CreatePushState, app: &mut App, code: KeyCode) -> ScreenAction {
    // Intercept Esc — return to Local screen, no changes.
    if code == KeyCode::Esc {
        return ScreenAction::SwitchTo(crate::tui::app::Screen::Local);
    }

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

    if let CreatePushState::PushModal {
        skill_name,
        skill_path,
        form,
    } = state
    {
        form.handle_input(key_event);

        match form.result() {
            FormResult::Submitted => {
                let json = form.to_json();
                let tags_raw = json["tags"].as_str().unwrap_or("").trim().to_string();
                let bump_str = json["bump"].as_str().unwrap_or("patch");
                let remote = json
                    .get("remote")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string());

                let bump = match bump_str {
                    "minor" => BumpChoice::Minor,
                    "major" => BumpChoice::Major,
                    "as-written" => BumpChoice::AsWritten,
                    _ => BumpChoice::Patch,
                };

                let skill = skill_name.clone();
                let path = skill_path.clone();

                // Write updated tags back to the local SKILL.md.
                if let Err(e) = update_tags_in_skill_md(&path, &tags_raw) {
                    let prior = CreatePushState::PushModal {
                        skill_name: skill.clone(),
                        skill_path: path.clone(),
                        form: Box::new(build_push_form_from_app(&skill, &tags_raw, app)),
                    };
                    *state = CreatePushState::Failed {
                        state: Box::new(prior),
                        message: format!("could not update tags: {}", e),
                    };
                    return ScreenAction::Stay;
                }

                *state = CreatePushState::ReadyToPush {
                    skill,
                    path,
                    remote,
                    bump,
                };
            }
            FormResult::Cancelled => {
                return ScreenAction::SwitchTo(crate::tui::app::Screen::Local);
            }
            FormResult::Active => {}
        }
    }
    ScreenAction::Stay
}

/// Handle keys for the `PushExistingForm` state.
///
/// Esc returns to Local. Submit writes updated tags to disk (deriving the path
/// from `app.project_root`) and transitions to `ReadyToPush`.
fn handle_push_existing_form(
    state: &mut CreatePushState,
    app: &mut App,
    code: KeyCode,
) -> ScreenAction {
    if code == KeyCode::Esc {
        return ScreenAction::SwitchTo(crate::tui::app::Screen::Local);
    }

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

    if let CreatePushState::PushExistingForm { skill, form } = state {
        form.handle_input(key_event);

        match form.result() {
            FormResult::Submitted => {
                let json = form.to_json();
                let tags_raw = json["tags"].as_str().unwrap_or("").trim().to_string();
                let bump_str = json["bump"].as_str().unwrap_or("patch");
                let remote = json
                    .get("remote")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string());

                let bump = match bump_str {
                    "minor" => BumpChoice::Minor,
                    "major" => BumpChoice::Major,
                    "as-written" => BumpChoice::AsWritten,
                    _ => BumpChoice::Patch,
                };

                let skill_name = skill.clone();
                let skill_path = app
                    .project_root
                    .join(".agents/skills")
                    .join(&skill_name)
                    .join("SKILL.md");

                if let Err(e) = update_tags_in_skill_md(&skill_path, &tags_raw) {
                    let remotes: Vec<String> = app.cfg.remotes.keys().cloned().collect();
                    let default_remote = app.cfg.default_remote().map(|(r, _)| r.to_string());
                    let tag_vec: Vec<String> =
                        tags_raw.split(',').map(|s| s.trim().to_string()).collect();
                    let prior = CreatePushState::PushExistingForm {
                        skill: skill_name.clone(),
                        form: build_push_existing_form(
                            &skill_name,
                            &tag_vec,
                            &remotes,
                            default_remote.as_deref(),
                        ),
                    };
                    *state = CreatePushState::Failed {
                        state: Box::new(prior),
                        message: format!("could not update tags: {}", e),
                    };
                    return ScreenAction::Stay;
                }

                *state = CreatePushState::ReadyToPush {
                    skill: skill_name,
                    path: skill_path,
                    remote,
                    bump,
                };
            }
            FormResult::Cancelled => {
                return ScreenAction::SwitchTo(crate::tui::app::Screen::Local);
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
                    // Direct-mode push: no PR URL to open.
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
// Tags update helper
// ---------------------------------------------------------------------------

/// Rewrite the `tags:` line in the YAML frontmatter of `path`.
///
/// Parses a comma-separated `tags_raw` string into a list, then rewrites the
/// SKILL.md file with the updated frontmatter.  If `tags_raw` is empty or
/// all-whitespace, tags are written as an empty list `tags: []`.
///
/// Does nothing and returns `Ok(())` if the file does not have YAML frontmatter.
fn update_tags_in_skill_md(path: &std::path::Path, tags_raw: &str) -> std::io::Result<()> {
    let raw = std::fs::read_to_string(path)?;

    let trimmed = raw.trim_start_matches('\u{feff}');
    let Some(rest) = trimmed.strip_prefix("---\n") else {
        // Not a frontmatter skill — leave it alone.
        return Ok(());
    };
    let Some((yaml, body)) = rest.split_once("\n---\n") else {
        return Ok(());
    };

    let new_tags: Vec<String> = tags_raw
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    let mut doc: serde_yaml::Value = serde_yaml::from_str(yaml).map_err(|e| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("invalid frontmatter YAML: {e}"),
        )
    })?;

    if let serde_yaml::Value::Mapping(ref mut map) = doc {
        let tag_key = serde_yaml::Value::String("tags".into());
        let tag_val = serde_yaml::Value::Sequence(
            new_tags
                .iter()
                .map(|t| serde_yaml::Value::String(t.clone()))
                .collect(),
        );
        map.insert(tag_key, tag_val);
    }

    let new_yaml = serde_yaml::to_string(&doc).map_err(|e| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("could not serialize frontmatter: {e}"),
        )
    })?;

    let new_content = format!("---\n{}\n---\n{}", new_yaml.trim_end(), body);
    std::fs::write(path, new_content)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Bulk push helpers
// ---------------------------------------------------------------------------

/// Initialise a `BulkPushing` state for `[u]` (quick push, Patch bump, default remote).
///
/// Defers the first `BlockingAction::Push` immediately.  Subsequent pushes are
/// advanced by the worker completion handler in `tui/mod.rs`.
pub fn bulk_quick_push(app: &mut App, names: Vec<String>) {
    if names.is_empty() {
        return;
    }
    let mut remaining: VecDeque<String> = names.into();
    let current = remaining.pop_front().expect("non-empty checked above");
    let total = remaining.len() + 1; // current + remaining
    app.create_push = CreatePushState::BulkPushing {
        remaining,
        total,
        current: current.clone(),
        bump: BumpChoice::Patch,
        remote: None,
        results: Vec::new(),
        spinner: Spinner::default(),
        started_at: Instant::now(),
    };
    app.push_form_ready = true;
    app.defer_blocking_action(BlockingAction::Push {
        skill: current,
        remote: None,
        bump: BumpKind::Patch,
    });
}

/// Initialise a `BulkPushing` state for `[U]` (form-based push with user-chosen bump/remote).
///
/// Like [`bulk_quick_push`] but uses the bump level and remote from the submitted push form.
pub fn bulk_form_push(app: &mut App, names: Vec<String>, bump: BumpChoice, remote: Option<String>) {
    if names.is_empty() {
        return;
    }
    let mut remaining: VecDeque<String> = names.into();
    let current = remaining.pop_front().expect("non-empty checked above");
    let total = remaining.len() + 1;
    let bump_kind = bump.as_bump_kind();
    app.create_push = CreatePushState::BulkPushing {
        remaining,
        total,
        current: current.clone(),
        bump,
        remote: remote.clone(),
        results: Vec::new(),
        spinner: Spinner::default(),
        started_at: Instant::now(),
    };
    app.push_form_ready = true;
    app.defer_blocking_action(BlockingAction::Push {
        skill: current,
        remote,
        bump: bump_kind,
    });
}

/// Advance the bulk-push state machine after a push completes.
///
/// Called from the TUI event loop once the worker returns a Push result.
/// Appends the result to `results`, pops the next skill from `remaining`,
/// defers another `BlockingAction::Push`, or transitions to `BulkDone` when
/// `remaining` is empty.
pub fn advance_bulk_push(app: &mut App, result: Result<PushOutcome, String>) {
    // Pull state out temporarily.
    let old = std::mem::replace(
        &mut app.create_push,
        CreatePushState::Form(build_create_form(&[])),
    );

    if let CreatePushState::BulkPushing {
        mut remaining,
        total,
        current,
        bump,
        remote,
        mut results,
        ..
    } = old
    {
        results.push(BulkResult {
            skill: current,
            outcome: result,
        });

        if let Some(next) = remaining.pop_front() {
            let bump_kind = bump.as_bump_kind();
            app.defer_blocking_action(BlockingAction::Push {
                skill: next.clone(),
                remote: remote.clone(),
                bump: bump_kind,
            });
            app.create_push = CreatePushState::BulkPushing {
                remaining,
                total,
                current: next,
                bump,
                remote,
                results,
                spinner: Spinner::default(),
                started_at: Instant::now(),
            };
        } else {
            app.create_push = CreatePushState::BulkDone(results);
        }
    }
    // Else: not in BulkPushing — placeholder Form state remains (no-op fallback).
}

// ---------------------------------------------------------------------------
// State transition helpers
// ---------------------------------------------------------------------------

fn on_save(state: &mut CreatePushState, app: &mut App, name: &str) {
    // Write a skeleton SKILL.md for the user to edit.
    let skill_dir = app.project_root.join(".agents/skills").join(name);
    let skill_md_path = skill_dir.join("SKILL.md");

    let write_result = (|| -> std::io::Result<()> {
        std::fs::create_dir_all(&skill_dir)?;
        if !skill_md_path.exists() {
            let template = format!(
                "---\nname: {name}\ndescription: \"\"\nversion: 0.1.0\ntags: []\n---\n\n# {name}\n\nDescribe what this skill does.\n"
            );
            std::fs::write(&skill_md_path, template)?;
        }
        Ok(())
    })();

    match write_result {
        Ok(()) => match crate::tui::editor::run_editor(&skill_md_path) {
            Ok(()) => {
                *state = CreatePushState::ReadyToValidate {
                    skill: name.to_string(),
                    path: skill_md_path,
                };
            }
            Err(e) => {
                *state = CreatePushState::Failed {
                    state: Box::new(CreatePushState::Form(build_create_form_from_app(app))),
                    message: format!("editor: {}", e),
                };
            }
        },
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

    *state = CreatePushState::Pushing {
        skill: skill.clone(),
        remote: remote.clone(),
        bump,
        started_at: Instant::now(),
        spinner: Spinner::default(),
    };

    app.defer_blocking_action(BlockingAction::Push {
        skill,
        remote,
        bump: bump.as_bump_kind(),
    });
}

/// Handle keys for the `BulkPushForm` state.
///
/// Esc returns to Local. Submit initiates `BulkPushing` via `bulk_form_push`.
fn handle_bulk_push_form(
    state: &mut CreatePushState,
    app: &mut App,
    code: KeyCode,
) -> ScreenAction {
    if code == KeyCode::Esc {
        return ScreenAction::SwitchTo(crate::tui::app::Screen::Local);
    }

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

    if let CreatePushState::BulkPushForm { skill_names, form } = state {
        form.handle_input(key_event);

        match form.result() {
            FormResult::Submitted => {
                let json = form.to_json();
                let bump_str = json["bump"].as_str().unwrap_or("patch");
                let remote = json
                    .get("remote")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string());

                let bump = match bump_str {
                    "minor" => BumpChoice::Minor,
                    "major" => BumpChoice::Major,
                    "as-written" => BumpChoice::AsWritten,
                    _ => BumpChoice::Patch,
                };

                let names = skill_names.clone();
                bulk_form_push(app, names, bump, remote);
            }
            FormResult::Cancelled => {
                return ScreenAction::SwitchTo(crate::tui::app::Screen::Local);
            }
            FormResult::Active => {}
        }
    }
    ScreenAction::Stay
}

fn handle_bulk_done(state: &mut CreatePushState, _app: &mut App, code: KeyCode) -> ScreenAction {
    match code {
        KeyCode::Esc | KeyCode::Char('b') => ScreenAction::SwitchTo(crate::tui::app::Screen::Local),
        _ => {
            // Clear the BulkDone on any other key and return to Local.
            let _ = state;
            ScreenAction::Stay
        }
    }
}

fn dismiss_failure(state: &mut CreatePushState, app: &mut App) {
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
        CreatePushState::PushModal { form, .. } => form.render(area, frame.buffer_mut()),
        CreatePushState::PushExistingForm { form, .. } => form.render(area, frame.buffer_mut()),
        CreatePushState::ScaffoldRunning => {
            render_placeholder(frame, area, "Creating scaffold...");
        }
        CreatePushState::Editing { skill, .. } => {
            render_placeholder(
                frame,
                area,
                &format!("Editing {} in $EDITOR\u{2026}", skill),
            );
        }
        CreatePushState::ReadyToValidate { skill, .. } => {
            render_ready_to_validate(frame, area, skill);
        }
        CreatePushState::Validating { skill, .. } => {
            render_placeholder(frame, area, &format!("Validating {}\u{2026}", skill));
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
        CreatePushState::BulkPushForm { form, .. } => form.render(area, frame.buffer_mut()),
        CreatePushState::BulkPushing {
            current,
            total,
            results,
            spinner,
            ..
        } => {
            render_bulk_pushing(frame, area, current, *total, results.len(), spinner);
        }
        CreatePushState::BulkDone(results) => {
            render_bulk_done(frame, area, results);
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
            Span::styled("  \u{2717} ", Style::default().fg(Color::Red)),
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
            "  Please wait \u{2014} this may take a few seconds.",
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
            "  \u{2714} Pushed {} v{} \u{2192} {}",
            outcome.skill, outcome.version, outcome.remote
        ),
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD),
    )]);

    let lines = if outcome.pr_url.is_empty() {
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
            Line::from(vec![Span::styled("  [b] back to dashboard", theme::dim())]),
        ]
    } else {
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

fn render_bulk_pushing(
    frame: &mut Frame,
    area: Rect,
    current: &str,
    total: usize,
    done: usize,
    spinner: &Spinner,
) {
    let outer = Block::default().borders(Borders::ALL).title(Span::styled(
        format!(" Bulk Push ({}/{}) ", done, total),
        theme::accent(),
    ));
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled(format!("  {} ", spinner.frame()), theme::accent()),
            Span::raw(format!(
                "Pushing skill {}… ({}/{} done)",
                current, done, total
            )),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "  Please wait \u{2014} pushing sequentially.",
            theme::dim(),
        )),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_bulk_done(frame: &mut Frame, area: Rect, results: &[BulkResult]) {
    let ok_count = results.iter().filter(|r| r.outcome.is_ok()).count();
    let total = results.len();
    let title = format!(" Bulk Push Done ({}/{} succeeded) ", ok_count, total);

    let outer = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(title, theme::accent()));
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let mut lines = vec![Line::from("")];
    for r in results {
        let line = match &r.outcome {
            Ok(outcome) => {
                let pr = if outcome.pr_url.is_empty() {
                    format!(
                        "direct push at {}",
                        &outcome.commit_sha[..8.min(outcome.commit_sha.len())]
                    )
                } else {
                    outcome.pr_url.clone()
                };
                Line::from(vec![
                    Span::styled("  \u{2714} ", Style::default().fg(Color::Green)),
                    Span::raw(format!("{} v{} \u{2192} {}", r.skill, outcome.version, pr)),
                ])
            }
            Err(e) => Line::from(vec![
                Span::styled("  \u{2717} ", Style::default().fg(Color::Red)),
                Span::raw(format!("{}: {}", r.skill, e)),
            ]),
        };
        lines.push(line);
    }
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::raw("  "),
        Span::styled(
            format!("pushed {} of {}", ok_count, total),
            Style::default().add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  [Esc] / [b] back to Local",
        theme::dim(),
    )));

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
            std::path::PathBuf::from("/tmp"),
            None,
        )
    }

    // -- BumpChoice tests --

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

    // -- PushModal form builder tests --

    #[test]
    fn push_form_tags_pre_filled() {
        use quay_core::config::MirrorRoot;
        use quay_core::scanner::{LocalLocation, LocalSkill, ScanStatus, SkillFormat, SkillMeta};

        let meta = SkillMeta {
            name: "my-skill".to_string(),
            description: "desc".to_string(),
            version: "0.1.0".to_string(),
            tags: vec!["foo".to_string(), "bar".to_string()],
            format: SkillFormat::Frontmatter,
        };
        let loc = LocalLocation {
            root: MirrorRoot::Agents,
            path: std::path::PathBuf::from("/tmp/.agents/skills/my-skill/SKILL.md"),
            sha256: "abc".to_string(),
        };
        let skill = LocalSkill {
            meta,
            locations: vec![loc],
            status: ScanStatus::Local,
        };

        let tags_initial = skill.meta.tags.join(", ");
        let form = build_push_form(&skill.meta.name, &tags_initial, &[]);
        let json = form.to_json();
        assert_eq!(
            json["tags"].as_str().unwrap_or(""),
            "foo, bar",
            "tags field must be pre-filled from skill frontmatter"
        );
    }

    #[test]
    fn push_form_title_contains_skill_name() {
        let form = build_push_form("csv-parser", "data, csv", &[]);
        let mut term = Terminal::new(TestBackend::new(80, 24)).unwrap();
        let app = make_app();
        let state = CreatePushState::PushModal {
            skill_name: "csv-parser".to_string(),
            skill_path: std::path::PathBuf::from("/tmp/x/SKILL.md"),
            form: Box::new(form),
        };
        term.draw(|f| render(f, &app, f.area(), &state)).unwrap();
        let buf = term.backend().buffer().clone();
        let content: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(
            content.contains("csv-parser"),
            "rendered buffer must contain skill name; got first 200 chars: {}",
            &content[..content.len().min(200)]
        );
    }

    #[test]
    fn push_form_no_name_or_description_fields() {
        let form = build_push_form("my-skill", "", &[]);
        let json = form.to_json();
        assert!(
            json.get("name").is_none(),
            "push form must not have a 'name' field"
        );
        assert!(
            json.get("description").is_none(),
            "push form must not have a 'description' field"
        );
        assert!(
            json.get("tags").is_some(),
            "push form must have a 'tags' field"
        );
        assert!(
            json.get("bump").is_some(),
            "push form must have a 'bump' field"
        );
    }

    #[test]
    fn update_tags_roundtrips_in_skill_md() {
        use assert_fs::prelude::*;
        let dir = assert_fs::TempDir::new().unwrap();
        let skill_md = dir.child("SKILL.md");
        skill_md
            .write_str(
                "---\nname: my-skill\ndescription: Test.\nversion: 0.1.0\ntags:\n  - old\n---\nbody\n",
            )
            .unwrap();

        update_tags_in_skill_md(skill_md.path(), "foo, bar").unwrap();

        let written = std::fs::read_to_string(skill_md.path()).unwrap();
        let meta = quay_core::scanner::parse_skill_metadata(&written, skill_md.path());
        assert_eq!(
            meta.tags,
            vec!["foo".to_string(), "bar".to_string()],
            "updated tags must be readable back from frontmatter"
        );
    }

    #[test]
    fn update_tags_preserves_existing_version() {
        // The version bump is performed by the pusher on push, not by
        // update_tags_in_skill_md. Verify the version field is preserved so
        // the pusher can apply its own bump correctly.
        use assert_fs::prelude::*;
        let dir = assert_fs::TempDir::new().unwrap();
        let skill_md = dir.child("SKILL.md");
        skill_md
            .write_str("---\nname: s\ndescription: d.\nversion: 0.1.0\ntags: []\n---\nbody\n")
            .unwrap();

        update_tags_in_skill_md(skill_md.path(), "newtag").unwrap();

        let written = std::fs::read_to_string(skill_md.path()).unwrap();
        let meta = quay_core::scanner::parse_skill_metadata(&written, skill_md.path());
        assert_eq!(
            meta.version, "0.1.0",
            "update_tags must preserve the existing version"
        );
    }

    #[test]
    fn push_modal_esc_returns_to_local_screen() {
        let mut app = make_app();
        app.create_push = CreatePushState::PushModal {
            skill_name: "s".into(),
            skill_path: std::path::PathBuf::from("/tmp/s/SKILL.md"),
            form: Box::new(build_push_form("s", "", &[])),
        };
        let action = handle_key(&mut app, KeyCode::Esc);
        assert!(
            matches!(
                action,
                ScreenAction::SwitchTo(crate::tui::app::Screen::Local)
            ),
            "Esc from PushModal must return to Local screen"
        );
    }

    #[test]
    fn is_push_modal_discriminant() {
        let modal = CreatePushState::PushModal {
            skill_name: "x".into(),
            skill_path: std::path::PathBuf::from("/tmp"),
            form: Box::new(build_push_form("x", "", &[])),
        };
        assert!(modal.is_push_modal());
        let form = CreatePushState::Form(build_create_form(&[]));
        assert!(!form.is_push_modal());
    }

    // -- Spinner test --

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
        // Tab once: name -> description.
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
    fn paste_into_push_modal_tags_field() {
        let mut state = CreatePushState::PushModal {
            skill_name: "s".into(),
            skill_path: std::path::PathBuf::from("/tmp/s/SKILL.md"),
            form: Box::new(build_push_form("s", "", &[])),
        };
        handle_paste(&mut state, "rust, cli");
        if let CreatePushState::PushModal { form, .. } = &state {
            let json = form.to_json();
            assert_eq!(json["tags"].as_str().unwrap_or(""), "rust, cli");
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
        let mut app = App::new(cfg, dir.path().to_path_buf(), None);

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
        let mut app = App::new(cfg, dir.path().to_path_buf(), None);

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
        let mut app = App::new(cfg, dir.path().to_path_buf(), None);

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
        let mut app = App::new(cfg, std::path::PathBuf::from("/tmp"), None);
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
