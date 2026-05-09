//! Settings → Profiles tab.

use crate::config_io::{read_user_file, write_user_file};
use crate::tui::app::{App, ScreenAction};
use crate::tui::theme;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use quay_core::ProfileFile;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph};
use ratatui::Frame;
use ratatui_form::{Form, FormResult, Pattern};

// ── State structs ──────────────────────────────────────────────────────────────

pub struct ProfilesState {
    pub list_state: ListState,
    pub modal: ModalState,
}

impl Default for ProfilesState {
    fn default() -> Self {
        Self {
            list_state: ListState::default(),
            modal: ModalState::Closed,
        }
    }
}

/// `Form` does not implement `Debug`, so we implement it manually.
impl std::fmt::Debug for ProfilesState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProfilesState")
            .field("list_state", &self.list_state)
            .field("modal", &self.modal)
            .finish()
    }
}

/// Modal state for the add/edit profile form.
#[derive(Default)]
pub enum ModalState {
    /// No modal open.
    #[default]
    Closed,
    /// Add or edit a profile.  `editing` is `Some(name)` when editing an
    /// existing profile, `None` when adding a new one.
    ///
    /// The `Form` is boxed to avoid a large enum variant (clippy::large_enum_variant).
    AddOrEdit {
        editing: Option<String>,
        form: Box<Form>,
    },
    /// Confirm deletion of the named profile.
    ConfirmDelete(String),
}

/// `Form` does not implement `Debug`, so we implement it manually.
impl std::fmt::Debug for ModalState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ModalState::Closed => write!(f, "Closed"),
            ModalState::AddOrEdit { editing, .. } => f
                .debug_struct("AddOrEdit")
                .field("editing", editing)
                .field("form", &"Form(...)")
                .finish(),
            ModalState::ConfirmDelete(name) => f.debug_tuple("ConfirmDelete").field(name).finish(),
        }
    }
}

// ── Form builder ──────────────────────────────────────────────────────────────

/// Build the add/edit profile form.
///
/// When `initial` is `Some`, pre-fills the name and email fields.
fn build_profile_modal_form(initial: Option<&ProfileFile>, initial_name: Option<&str>) -> Form {
    let name_initial = initial_name.unwrap_or("");
    let email_initial = initial.and_then(|p| p.user.email.as_deref()).unwrap_or("");
    Form::builder()
        .title(if initial.is_some() {
            "Edit profile"
        } else {
            "Add profile"
        })
        .style(crate::tui::form_theme::dark())
        .text("name", "Name")
        .required()
        .validator(Box::new(Pattern::new(
            r"^[a-z0-9-]+$",
            "lowercase letters, digits, hyphens only",
        )))
        .placeholder("org-a")
        .initial_value(name_initial)
        .done()
        .text("email", "Email")
        .required()
        .initial_value(email_initial)
        .done()
        .build()
}

// ── Paste handler ─────────────────────────────────────────────────────────────

/// Forward pasted text into the form when the add/edit modal is open.
///
/// Silently dropped when no form modal is open (browsing or confirm-delete).
pub fn handle_paste(state: &mut ProfilesState, s: &str) {
    if let ModalState::AddOrEdit { form, .. } = &mut state.modal {
        let events = crate::tui::paste_to_key_events(s);
        for ev in events {
            form.handle_input(ev);
        }
    }
}

// ── Key handlers ──────────────────────────────────────────────────────────────

/// Returns `true` if the profiles tab has a form modal open.
///
/// Called by `settings::handle_key` to decide whether Tab should cycle settings
/// tabs or be forwarded to the form.
pub fn has_active_modal(state: &ProfilesState) -> bool {
    !matches!(state.modal, ModalState::Closed)
}

pub fn handle_key(app: &mut App, code: KeyCode) -> ScreenAction {
    match &app.settings.profiles.modal {
        ModalState::Closed => handle_browsing(app, code),
        ModalState::AddOrEdit { .. } => handle_form(app, code),
        ModalState::ConfirmDelete(_) => handle_confirm(app, code),
    }
}

fn handle_browsing(app: &mut App, code: KeyCode) -> ScreenAction {
    match code {
        KeyCode::Down | KeyCode::Char('j') => {
            let max = profile_count(app).saturating_sub(1);
            let i = app.settings.profiles.list_state.selected().unwrap_or(0);
            app.settings
                .profiles
                .list_state
                .select(Some(i.saturating_add(1).min(max)));
        }
        KeyCode::Up | KeyCode::Char('k') => {
            let i = app.settings.profiles.list_state.selected().unwrap_or(0);
            app.settings
                .profiles
                .list_state
                .select(Some(i.saturating_sub(1)));
        }
        KeyCode::Char('a') => {
            app.settings.profiles.modal = ModalState::AddOrEdit {
                editing: None,
                form: Box::new(build_profile_modal_form(None, None)),
            };
        }
        KeyCode::Char('r') => {
            if let Some(name) = selected_name(app) {
                let path = app.user_config_path.as_deref();
                let file = read_user_file(path).unwrap_or_default();
                let profile = file.profiles.get(&name).cloned();
                app.settings.profiles.modal = ModalState::AddOrEdit {
                    form: Box::new(build_profile_modal_form(profile.as_ref(), Some(&name))),
                    editing: Some(name),
                };
            }
        }
        KeyCode::Char('d') => {
            if let Some(name) = selected_name(app) {
                app.settings.profiles.modal = ModalState::ConfirmDelete(name);
            }
        }
        KeyCode::Char('u') | KeyCode::Enter => {
            if let Some(name) = selected_name(app) {
                if let Err(e) = use_profile(app, &name) {
                    app.set_status(format!("error: {}", e));
                } else {
                    app.set_status(format!("active profile is now: {}", name));
                }
            }
        }
        _ => {}
    }
    ScreenAction::Stay
}

fn handle_form(app: &mut App, code: KeyCode) -> ScreenAction {
    // Intercept Esc before delegating to the form so we can close the modal.
    if code == KeyCode::Esc {
        app.settings.profiles.modal = ModalState::Closed;
        return ScreenAction::Stay;
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

    if let ModalState::AddOrEdit { form, editing } = &mut app.settings.profiles.modal {
        form.handle_input(key_event);

        if matches!(form.result(), FormResult::Submitted) {
            let json = form.to_json();
            let name = json["name"].as_str().unwrap_or("").trim().to_string();
            let email = json["email"].as_str().unwrap_or("").trim().to_string();
            let editing_name = editing.clone();

            match submit_profile(app, &name, &email, editing_name.as_deref()) {
                Ok(msg) => {
                    app.set_status(msg);
                    app.settings.profiles.modal = ModalState::Closed;
                }
                Err(e) => app.set_status(format!("error: {}", e)),
            }
        } else if matches!(form.result(), FormResult::Cancelled) {
            app.settings.profiles.modal = ModalState::Closed;
        }
    }
    ScreenAction::Stay
}

fn handle_confirm(app: &mut App, code: KeyCode) -> ScreenAction {
    let name = match &app.settings.profiles.modal {
        ModalState::ConfirmDelete(name) => name.clone(),
        _ => return ScreenAction::Stay,
    };
    match code {
        KeyCode::Char('y') | KeyCode::Enter => match submit_delete(app, &name) {
            Ok(_) => {
                app.set_status(format!("removed profile '{}'", name));
                app.settings.profiles.modal = ModalState::Closed;
            }
            Err(e) => {
                app.set_status(format!("error: {}", e));
                app.settings.profiles.modal = ModalState::Closed;
            }
        },
        KeyCode::Esc | KeyCode::Char('n') => {
            app.settings.profiles.modal = ModalState::Closed;
        }
        _ => {}
    }
    ScreenAction::Stay
}

// ── Data helpers ──────────────────────────────────────────────────────────────

fn profile_count(app: &App) -> usize {
    let path = app.user_config_path.as_deref();
    read_user_file(path).map(|f| f.profiles.len()).unwrap_or(0)
}

fn selected_name(app: &App) -> Option<String> {
    let path = app.user_config_path.as_deref();
    let file = read_user_file(path).ok()?;
    let i = app.settings.profiles.list_state.selected().unwrap_or(0);
    file.profiles.keys().nth(i).cloned()
}

fn submit_profile(
    app: &mut App,
    name: &str,
    email: &str,
    editing: Option<&str>,
) -> Result<String, Box<dyn std::error::Error>> {
    let path = app
        .user_config_path
        .as_deref()
        .ok_or("--user-config required")?;
    let mut file = read_user_file(Some(path))?;

    if let Some(old_name) = editing {
        // Rename + update email.
        if name != old_name && file.profiles.contains_key(name) {
            return Err(format!("profile '{}' already exists", name).into());
        }
        let mut p = file
            .profiles
            .remove(old_name)
            .ok_or(format!("profile '{}' missing", old_name))?;
        p.user.email = if email.is_empty() {
            None
        } else {
            Some(email.into())
        };
        file.profiles.insert(name.to_string(), p);
        if file.active_profile.as_deref() == Some(old_name) {
            file.active_profile = Some(name.to_string());
        }
        write_user_file(path, &file)?;
        Ok("profile updated".to_string())
    } else {
        // Add new profile.
        if file.profiles.contains_key(name) {
            return Err(format!("profile '{}' already exists", name).into());
        }
        let mut p = ProfileFile::default();
        if !email.is_empty() {
            p.user.email = Some(email.into());
        }
        file.profiles.insert(name.to_string(), p);
        if file.active_profile.is_none() {
            file.active_profile = Some(name.to_string());
        }
        write_user_file(path, &file)?;
        Ok("profile added".to_string())
    }
}

fn submit_delete(app: &mut App, name: &str) -> Result<(), Box<dyn std::error::Error>> {
    let path = app
        .user_config_path
        .as_deref()
        .ok_or("--user-config required")?;
    let mut file = read_user_file(Some(path))?;
    if file.profiles.len() == 1 {
        return Err("cannot remove the only profile".into());
    }
    if !file.profiles.contains_key(name) {
        return Err(format!("profile '{}' not found", name).into());
    }
    file.profiles.remove(name);
    if file.active_profile.as_deref() == Some(name) {
        file.active_profile = file.profiles.keys().next().cloned();
    }
    write_user_file(path, &file)?;
    Ok(())
}

fn use_profile(app: &mut App, name: &str) -> Result<(), Box<dyn std::error::Error>> {
    let path = app
        .user_config_path
        .as_deref()
        .ok_or("--user-config required")?;
    let mut file = read_user_file(Some(path))?;
    if !file.profiles.contains_key(name) {
        return Err(format!("profile '{}' not found", name).into());
    }
    file.active_profile = Some(name.into());
    write_user_file(path, &file)?;
    Ok(())
}

// ── Render ────────────────────────────────────────────────────────────────────

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(area);

    let path = app.user_config_path.as_deref();
    let file = read_user_file(path).unwrap_or_default();
    let active = file.active_profile.clone();

    let items: Vec<ListItem> = file
        .profiles
        .iter()
        .map(|(name, p)| {
            let marker = if active.as_deref() == Some(name.as_str()) {
                "▶ "
            } else {
                "  "
            };
            ListItem::new(Line::from(format!(
                "{}{}\t{}",
                marker,
                name,
                p.user.email.as_deref().unwrap_or("(no email)"),
            )))
        })
        .collect();
    let mut list_state = app.settings.profiles.list_state.clone();
    let title = format!(" Profiles ({}) ", file.profiles.len());
    frame.render_stateful_widget(
        List::new(items)
            .block(Block::default().borders(Borders::ALL).title(title))
            .highlight_style(theme::selected()),
        cols[0],
        &mut list_state,
    );

    let hints = vec![
        Line::from("[a] add  [r] rename  [d] delete"),
        Line::from("[u] / Enter — set active"),
        Line::from("[Tab] next tab  [q] quit"),
    ];
    frame.render_widget(
        Paragraph::new(hints)
            .style(Style::default())
            .block(Block::default().borders(Borders::ALL).title(" Actions ")),
        cols[1],
    );

    // Modal overlay.
    match &app.settings.profiles.modal {
        ModalState::AddOrEdit { form, .. } => {
            let modal_area = centered_rect(area, 50, 50);
            frame.render_widget(Clear, modal_area);
            form.render(modal_area, frame.buffer_mut());
        }
        ModalState::ConfirmDelete(name) => render_confirm_modal(frame, area, name),
        ModalState::Closed => {}
    }
}

fn render_confirm_modal(frame: &mut Frame, area: Rect, name: &str) {
    let modal_area = centered_rect(area, 50, 20);
    frame.render_widget(Clear, modal_area);
    let block = Block::default().borders(Borders::ALL).title(" Confirm ");
    let inner = block.inner(modal_area);
    frame.render_widget(block, modal_area);
    let lines = vec![
        Line::from(format!("Delete profile '{}'?", name)),
        Line::from(""),
        Line::from("y / Enter — yes   n / Esc — no"),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}

fn centered_rect(area: Rect, percent_x: u16, percent_y: u16) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use assert_fs::prelude::*;
    use quay_core::{Config, Lockfile};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn fixture_app() -> (App, assert_fs::TempDir) {
        let dir = assert_fs::TempDir::new().unwrap();
        let user_path = dir.child("user.toml");
        let mut p = ProfileFile::default();
        p.user.email = Some("e@work".into());
        let mut file = quay_core::UserConfigFile {
            active_profile: Some("work".into()),
            ..Default::default()
        };
        file.profiles.insert("work".into(), p);
        write_user_file(user_path.path(), &file).unwrap();

        let mut a = App::new(
            Config::default(),
            Lockfile::default(),
            dir.path().to_path_buf(),
            Some(user_path.path().to_path_buf()),
        );
        a.current_screen = crate::tui::app::Screen::Settings;
        (a, dir)
    }

    #[test]
    fn renders_profile_row_and_active_marker() {
        let (a, _dir) = fixture_app();
        let mut term = Terminal::new(TestBackend::new(80, 20)).unwrap();
        term.draw(|f| crate::tui::draw(f, &a)).unwrap();
        let dump: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(dump.contains("Profiles"));
        assert!(dump.contains("▶ work"), "dump: {}", dump);
    }

    #[test]
    fn add_form_submits_new_profile_to_disk() {
        let (mut a, dir) = fixture_app();
        // Open add form.
        handle_key(&mut a, KeyCode::Char('a'));
        assert!(matches!(
            a.settings.profiles.modal,
            ModalState::AddOrEdit { editing: None, .. }
        ));

        // Type name then Tab to email.
        let form_events = |a: &mut App, code: KeyCode| handle_key(a, code);

        // Feed name characters.
        for c in "personal".chars() {
            form_events(&mut a, KeyCode::Char(c));
        }
        // Tab to email.
        form_events(&mut a, KeyCode::Tab);
        for c in "e@home".chars() {
            form_events(&mut a, KeyCode::Char(c));
        }
        // Tab to Submit button.
        form_events(&mut a, KeyCode::Tab);
        // Press Enter to submit.
        form_events(&mut a, KeyCode::Enter);

        // Assert disk.
        let written = std::fs::read_to_string(dir.child("user.toml").path()).unwrap();
        assert!(written.contains("[profiles.personal"), "wrote: {}", written);
        assert!(written.contains("e@home"), "wrote: {}", written);
        assert!(
            matches!(a.settings.profiles.modal, ModalState::Closed),
            "modal should be closed after submit"
        );
    }

    #[test]
    fn paste_into_email_field_when_adding() {
        let (mut a, _dir) = fixture_app();
        // Open add form.
        handle_key(&mut a, KeyCode::Char('a'));
        assert!(matches!(
            a.settings.profiles.modal,
            ModalState::AddOrEdit { .. }
        ));
        // Tab from name to email.
        handle_key(&mut a, KeyCode::Tab);
        // Paste an email.
        handle_paste(&mut a.settings.profiles, "user@example.com");
        // Verify form value via to_json.
        if let ModalState::AddOrEdit { form, .. } = &a.settings.profiles.modal {
            let json = form.to_json();
            assert_eq!(
                json["email"].as_str().unwrap_or(""),
                "user@example.com",
                "paste should fill email field"
            );
        } else {
            panic!("modal should still be open");
        }
    }

    #[test]
    fn paste_noop_when_modal_closed() {
        let (mut a, _dir) = fixture_app();
        assert!(matches!(a.settings.profiles.modal, ModalState::Closed));
        // Should not panic.
        handle_paste(&mut a.settings.profiles, "should-not-appear");
    }

    #[test]
    fn delete_confirm_removes_profile() {
        let (mut a, dir) = fixture_app();
        // Pre-add a second profile so deletion is allowed.
        let mut file = read_user_file(a.user_config_path.as_deref()).unwrap();
        file.profiles
            .insert("personal".into(), ProfileFile::default());
        write_user_file(a.user_config_path.as_deref().unwrap(), &file).unwrap();

        // Move selection to index 1 (work sorts after personal alphabetically).
        handle_key(&mut a, KeyCode::Down);
        // Press d to enter confirm mode.
        handle_key(&mut a, KeyCode::Char('d'));
        assert!(matches!(
            a.settings.profiles.modal,
            ModalState::ConfirmDelete(_)
        ));
        handle_key(&mut a, KeyCode::Char('y'));
        let written = std::fs::read_to_string(dir.child("user.toml").path()).unwrap();
        assert!(!written.contains("[profiles.work"), "wrote: {}", written);
    }
}
