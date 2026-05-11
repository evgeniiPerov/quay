//! Settings → Remotes tab. Edits the active profile's remotes inside the user
//! config file.

use crate::config_io::{read_user_file, write_user_file};
use crate::tui::app::{App, BlockingAction, ScreenAction};
use crate::tui::screens::widgets::spinner::Spinner;
use crate::tui::theme;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use quay_core::{ConnectionStatus, ProviderKind, RemoteConfig};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph};
use ratatui::Frame;
use ratatui_form::{Form, FormResult};
use std::collections::HashMap;

// ── State structs ─────────────────────────────────────────────────────────────

pub struct RemotesState {
    pub list_state: ListState,
    pub modal: ModalState,
    /// Index of the remote currently being tested (spinner shown).
    pub testing_idx: Option<usize>,
    /// Per-row test results, keyed by row index.
    pub last_results: HashMap<usize, ConnectionStatus>,
    /// Spinner driven by the 250 ms tick clock while `testing_idx.is_some()`.
    pub spinner: Spinner,
}

impl Default for RemotesState {
    fn default() -> Self {
        Self {
            list_state: ListState::default(),
            modal: ModalState::Closed,
            testing_idx: None,
            last_results: HashMap::new(),
            spinner: Spinner::default(),
        }
    }
}

/// `Form` does not implement `Debug`, so we implement it manually.
impl std::fmt::Debug for RemotesState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RemotesState")
            .field("list_state", &self.list_state)
            .field("modal", &self.modal)
            .field("testing_idx", &self.testing_idx)
            .field("spinner", &self.spinner)
            .finish()
    }
}

/// Modal state for the add/edit remote form.
#[derive(Default)]
pub enum ModalState {
    /// No modal open.
    #[default]
    Closed,
    /// Add or edit a remote.  `editing` is `Some(name)` when editing an
    /// existing remote, `None` when adding a new one.
    ///
    /// The `Form` is boxed to avoid a large enum variant (clippy::large_enum_variant).
    AddOrEdit {
        editing: Option<String>,
        form: Box<Form>,
    },
    /// Confirm deletion of the named remote.
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

/// Returns true when the remotes tab has a form modal open.
pub fn has_active_modal(state: &RemotesState) -> bool {
    !matches!(state.modal, ModalState::Closed)
}

// ── Provider helpers ──────────────────────────────────────────────────────────

fn provider_kind_to_str(k: ProviderKind) -> &'static str {
    match k {
        ProviderKind::GitHub => "github",
        ProviderKind::GitHubEnterprise => "githubenterprise",
        ProviderKind::GitLab => "gitlab",
        ProviderKind::Bitbucket => "bitbucket",
        ProviderKind::AzureDevOps => "azuredevops",
    }
}

fn provider_str_to_kind(s: &str) -> Option<ProviderKind> {
    match s {
        "auto" => None,
        "github" => Some(ProviderKind::GitHub),
        "githubenterprise" => Some(ProviderKind::GitHubEnterprise),
        "gitlab" => Some(ProviderKind::GitLab),
        "bitbucket" => Some(ProviderKind::Bitbucket),
        "azuredevops" => Some(ProviderKind::AzureDevOps),
        _ => None,
    }
}

// ── Form builder ──────────────────────────────────────────────────────────────

fn build_remote_modal_form(
    initial_remote: Option<&RemoteConfig>,
    initial_name: Option<&str>,
) -> Form {
    let name_initial = initial_name.unwrap_or("");
    let url_initial = initial_remote.map(|r| r.url.as_str()).unwrap_or("");
    let provider_initial = initial_remote
        .and_then(|r| r.provider)
        .map(provider_kind_to_str)
        .unwrap_or("auto");
    let default_initial = initial_remote.map(|r| r.default).unwrap_or(false);
    let push_mode_initial = match initial_remote.map(|r| r.push_mode).unwrap_or_default() {
        quay_core::PushMode::Pr => "pr",
        quay_core::PushMode::Direct => "direct",
    };
    // Empty string in the form represents `None` (no override).
    let direct_branch_initial = initial_remote
        .and_then(|r| r.direct_branch.as_deref())
        .unwrap_or("");

    Form::builder()
        .title(if initial_remote.is_some() {
            "Edit remote"
        } else {
            "Add remote"
        })
        .style(crate::tui::form_theme::dark())
        .text("name", "Name")
        .required()
        .initial_value(name_initial)
        .done()
        .text("url", "Git URL")
        .required()
        .initial_value(url_initial)
        .done()
        .select("provider", "Provider")
        .options(vec![
            ("auto", "Auto-detect"),
            ("github", "GitHub"),
            ("githubenterprise", "GitHub Enterprise"),
            ("gitlab", "GitLab"),
            ("bitbucket", "Bitbucket"),
            ("azuredevops", "Azure DevOps"),
        ])
        .initial_value(provider_initial)
        .done()
        .select("push_mode", "Push mode")
        .options(vec![
            ("pr", "Open PR (default)"),
            ("direct", "Direct git push"),
        ])
        .initial_value(push_mode_initial)
        .done()
        .text("direct_branch", "Direct branch")
        .initial_value(direct_branch_initial)
        .done()
        .checkbox("default", "Default remote")
        .checked(default_initial)
        .done()
        .build()
}

fn push_mode_str_to_kind(s: &str) -> quay_core::PushMode {
    match s {
        "direct" => quay_core::PushMode::Direct,
        _ => quay_core::PushMode::Pr,
    }
}

// ── Paste handler ─────────────────────────────────────────────────────────────

/// Forward pasted text into the form when the add/edit modal is open.
///
/// Silently dropped when no form modal is open.
pub fn handle_paste(state: &mut RemotesState, s: &str) {
    if let ModalState::AddOrEdit { form, .. } = &mut state.modal {
        let events = crate::tui::paste_to_key_events(s);
        for ev in events {
            form.handle_input(ev);
        }
    }
}

// ── Key handlers ──────────────────────────────────────────────────────────────

pub fn handle_key(app: &mut App, code: KeyCode) -> ScreenAction {
    match &app.settings.remotes.modal {
        ModalState::Closed => handle_browsing(app, code),
        ModalState::AddOrEdit { .. } => handle_form(app, code),
        ModalState::ConfirmDelete(_) => handle_confirm(app, code),
    }
}

fn handle_browsing(app: &mut App, code: KeyCode) -> ScreenAction {
    match code {
        KeyCode::Down | KeyCode::Char('j') => {
            let max = remote_count(app).saturating_sub(1);
            let i = app.settings.remotes.list_state.selected().unwrap_or(0);
            app.settings
                .remotes
                .list_state
                .select(Some(i.saturating_add(1).min(max)));
        }
        KeyCode::Up | KeyCode::Char('k') => {
            let i = app.settings.remotes.list_state.selected().unwrap_or(0);
            app.settings
                .remotes
                .list_state
                .select(Some(i.saturating_sub(1)));
        }
        KeyCode::Char('a') => {
            app.settings.remotes.modal = ModalState::AddOrEdit {
                editing: None,
                form: Box::new(build_remote_modal_form(None, None)),
            };
        }
        KeyCode::Char('e') => {
            if let Some(name) = selected_name(app) {
                let remote = selected_remote(app);
                app.settings.remotes.modal = ModalState::AddOrEdit {
                    editing: Some(name.clone()),
                    form: Box::new(build_remote_modal_form(remote.as_ref(), Some(&name))),
                };
            }
        }
        KeyCode::Char('d') => {
            if let Some(name) = selected_name(app) {
                app.settings.remotes.modal = ModalState::ConfirmDelete(name);
            }
        }
        KeyCode::Char('s') => {
            if let Some(name) = selected_name(app) {
                if let Err(e) = set_default(app, &name) {
                    app.set_status(format!("error: {}", e));
                } else {
                    app.set_status(format!("default remote: {}", name));
                }
            }
        }
        KeyCode::Char('t') => {
            on_test_connection(app);
        }
        _ => {}
    }
    ScreenAction::Stay
}

/// Queue a live test-connection blocking action for the currently selected row.
fn on_test_connection(app: &mut App) {
    let idx = app.settings.remotes.list_state.selected().unwrap_or(0);
    let (url, kind) = match selected_remote(app) {
        Some(r) => (r.url.clone(), r.provider),
        None => return,
    };
    app.settings.remotes.testing_idx = Some(idx);
    app.settings.remotes.last_results.remove(&idx);
    app.defer_blocking_action(BlockingAction::TestConnection {
        url,
        kind,
        remote_idx: idx,
    });
}

fn handle_form(app: &mut App, code: KeyCode) -> ScreenAction {
    // Intercept Esc before delegating to the form so we can close the modal.
    if code == KeyCode::Esc {
        app.settings.remotes.modal = ModalState::Closed;
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

    if let ModalState::AddOrEdit { form, editing } = &mut app.settings.remotes.modal {
        form.handle_input(key_event);

        if matches!(form.result(), FormResult::Submitted) {
            let json = form.to_json();
            let name = json["name"].as_str().unwrap_or("").trim().to_string();
            let url = json["url"].as_str().unwrap_or("").trim().to_string();
            let provider = provider_str_to_kind(json["provider"].as_str().unwrap_or("auto"));
            let push_mode = push_mode_str_to_kind(json["push_mode"].as_str().unwrap_or("pr"));
            // Empty string in the form means "no override" (None).
            let direct_branch: Option<String> = json["direct_branch"]
                .as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            let default = json["default"].as_bool().unwrap_or(false);
            let editing_name = editing.clone();

            match submit_remote(
                app,
                &name,
                &url,
                provider,
                push_mode,
                direct_branch,
                default,
                editing_name.as_deref(),
            ) {
                Ok(msg) => {
                    app.set_status(msg);
                    app.settings.remotes.modal = ModalState::Closed;
                }
                Err(e) => app.set_status(format!("error: {}", e)),
            }
        } else if matches!(form.result(), FormResult::Cancelled) {
            app.settings.remotes.modal = ModalState::Closed;
        }
    }
    ScreenAction::Stay
}

fn handle_confirm(app: &mut App, code: KeyCode) -> ScreenAction {
    let name = match &app.settings.remotes.modal {
        ModalState::ConfirmDelete(name) => name.clone(),
        _ => return ScreenAction::Stay,
    };
    match code {
        KeyCode::Char('y') | KeyCode::Enter => match submit_delete(app, &name) {
            Ok(_) => {
                app.set_status(format!("removed remote '{}'", name));
                app.settings.remotes.modal = ModalState::Closed;
            }
            Err(e) => {
                app.set_status(format!("error: {}", e));
                app.settings.remotes.modal = ModalState::Closed;
            }
        },
        KeyCode::Esc | KeyCode::Char('n') => {
            app.settings.remotes.modal = ModalState::Closed;
        }
        _ => {}
    }
    ScreenAction::Stay
}

// ── Data helpers ──────────────────────────────────────────────────────────────

fn active_profile_name(app: &App) -> Option<String> {
    let path = app.user_config_path.as_deref()?;
    read_user_file(Some(path)).ok()?.active_profile
}

fn remote_count(app: &App) -> usize {
    let path = app.user_config_path.as_deref();
    let file = read_user_file(path).unwrap_or_default();
    let active = match file.active_profile.as_deref() {
        Some(s) => s,
        None => return 0,
    };
    file.profiles
        .get(active)
        .map(|p| p.remotes.len())
        .unwrap_or(0)
}

fn selected_name(app: &App) -> Option<String> {
    let path = app.user_config_path.as_deref()?;
    let file = read_user_file(Some(path)).ok()?;
    let active = file.active_profile.clone()?;
    let p = file.profiles.get(&active)?;
    let i = app.settings.remotes.list_state.selected().unwrap_or(0);
    p.remotes.keys().nth(i).cloned()
}

/// Return a clone of the selected remote's config, or `None`.
fn selected_remote(app: &App) -> Option<RemoteConfig> {
    let path = app.user_config_path.as_deref()?;
    let file = read_user_file(Some(path)).ok()?;
    let active = file.active_profile.clone()?;
    let p = file.profiles.get(&active)?;
    let i = app.settings.remotes.list_state.selected().unwrap_or(0);
    let key = p.remotes.keys().nth(i)?;
    p.remotes.get(key).cloned()
}

#[allow(clippy::too_many_arguments)]
fn submit_remote(
    app: &mut App,
    name: &str,
    url: &str,
    provider: Option<ProviderKind>,
    push_mode: quay_core::PushMode,
    direct_branch: Option<String>,
    default: bool,
    editing: Option<&str>,
) -> Result<String, Box<dyn std::error::Error>> {
    let path = app
        .user_config_path
        .as_deref()
        .ok_or("--user-config required")?;
    let mut file = read_user_file(Some(path))?;
    let active = active_profile_name(app).ok_or("no active profile")?;
    let p = file
        .profiles
        .get_mut(&active)
        .ok_or("active profile missing")?;

    if let Some(old_name) = editing {
        // Edit existing: remove old, insert updated.
        if name != old_name && p.remotes.contains_key(name) {
            return Err(format!("remote '{}' already exists", name).into());
        }
        let mut remote = p
            .remotes
            .remove(old_name)
            .ok_or(format!("remote '{}' missing", old_name))?;
        remote.url = url.to_string();
        remote.provider = provider;
        remote.push_mode = push_mode;
        remote.direct_branch = direct_branch;
        remote.default = default;
        p.remotes.insert(name.to_string(), remote);
        write_user_file(path, &file)?;
        Ok("remote updated".to_string())
    } else {
        // Add new remote.
        if p.remotes.contains_key(name) {
            return Err(format!("remote '{}' already exists", name).into());
        }
        p.remotes.insert(
            name.to_string(),
            RemoteConfig {
                url: url.to_string(),
                default: p.remotes.is_empty() || default,
                provider,
                push_mode,
                direct_branch,
            },
        );
        write_user_file(path, &file)?;
        Ok("remote added".to_string())
    }
}

fn submit_delete(app: &mut App, name: &str) -> Result<(), Box<dyn std::error::Error>> {
    let path = app
        .user_config_path
        .as_deref()
        .ok_or("--user-config required")?;
    let mut file = read_user_file(Some(path))?;
    let active = active_profile_name(app).ok_or("no active profile")?;
    let p = file
        .profiles
        .get_mut(&active)
        .ok_or("active profile missing")?;
    if p.remotes.remove(name).is_none() {
        return Err(format!("remote '{}' not found", name).into());
    }
    write_user_file(path, &file)?;
    Ok(())
}

fn set_default(app: &mut App, name: &str) -> Result<(), Box<dyn std::error::Error>> {
    let path = app
        .user_config_path
        .as_deref()
        .ok_or("--user-config required")?;
    let mut file = read_user_file(Some(path))?;
    let active = active_profile_name(app).ok_or("no active profile")?;
    let p = file
        .profiles
        .get_mut(&active)
        .ok_or("active profile missing")?;
    if !p.remotes.contains_key(name) {
        return Err(format!("remote '{}' not found", name).into());
    }
    for (n, r) in p.remotes.iter_mut() {
        r.default = n == name;
    }
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
    let title = match &active {
        Some(a) => format!(" Remotes — profile '{}' ", a),
        None => " Remotes — (no active profile) ".into(),
    };

    let remotes = &app.settings.remotes;
    let items: Vec<ListItem> = active
        .as_deref()
        .and_then(|a| file.profiles.get(a))
        .map(|p| {
            p.remotes
                .iter()
                .enumerate()
                .map(|(row_idx, (name, r))| {
                    let trailing = if Some(row_idx) == remotes.testing_idx {
                        remotes.spinner.frame().to_string()
                    } else if let Some(status) = remotes.last_results.get(&row_idx) {
                        match status {
                            ConnectionStatus::Ok {
                                registry_size_bytes,
                            } => format!("✓ {} B", registry_size_bytes),
                            ConnectionStatus::AuthFailed(_) => "✗ auth".into(),
                            ConnectionStatus::Unreachable(_) => "✗ unreachable".into(),
                            ConnectionStatus::NoRegistry(_) => "✗ no registry".into(),
                        }
                    } else if r.default {
                        "[default]".into()
                    } else {
                        String::new()
                    };
                    ListItem::new(Line::from(format!("{}\t{}  {}", name, r.url, trailing)))
                })
                .collect()
        })
        .unwrap_or_default();
    let mut list_state = app.settings.remotes.list_state.clone();
    frame.render_stateful_widget(
        List::new(items)
            .block(Block::default().borders(Borders::ALL).title(title))
            .highlight_style(theme::selected()),
        cols[0],
        &mut list_state,
    );

    let hints = vec![
        Line::from("[a] add  [e] edit  [d] delete"),
        Line::from("[s] set default  [t] test"),
        Line::from("[Tab] next tab  [q] quit"),
    ];
    frame.render_widget(
        Paragraph::new(hints).block(Block::default().borders(Borders::ALL).title(" Actions ")),
        cols[1],
    );

    match &app.settings.remotes.modal {
        ModalState::AddOrEdit { form, .. } => {
            let modal_area = centered_rect(area, 60, 60);
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
        Line::from(format!("Delete remote '{}'?", name)),
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

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use assert_fs::prelude::*;
    use quay_core::{Config, ProfileFile};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn fixture_app() -> (App, assert_fs::TempDir) {
        let dir = assert_fs::TempDir::new().unwrap();
        let user_path = dir.child("user.toml");
        let mut file = quay_core::UserConfigFile {
            active_profile: Some("work".into()),
            ..Default::default()
        };
        let mut p = ProfileFile::default();
        p.user.email = Some("e@work".into());
        p.remotes.insert(
            "primary".into(),
            RemoteConfig {
                url: "https://github.com/x/y.git".into(),
                default: true,
                provider: None,
                push_mode: quay_core::PushMode::default(),
                direct_branch: None,
            },
        );
        file.profiles.insert("work".into(), p);
        write_user_file(user_path.path(), &file).unwrap();

        let mut a = App::new(
            Config::default(),
            dir.path().to_path_buf(),
            Some(user_path.path().to_path_buf()),
        );
        a.current_screen = crate::tui::app::Screen::Settings;
        a.settings.tab = crate::tui::app::SettingsTab::Remotes;
        (a, dir)
    }

    // ── Paste handler tests ────────────────────────────────────────────────────

    #[test]
    fn paste_inserts_into_url_field_when_adding() {
        let (mut a, _dir) = fixture_app();
        handle_key(&mut a, KeyCode::Char('a'));
        assert!(matches!(
            a.settings.remotes.modal,
            ModalState::AddOrEdit { .. }
        ));
        // Tab from name to url.
        handle_key(&mut a, KeyCode::Tab);
        handle_paste(&mut a.settings.remotes, "git@github.com:org/skills.git");
        if let ModalState::AddOrEdit { form, .. } = &a.settings.remotes.modal {
            let json = form.to_json();
            assert_eq!(
                json["url"].as_str().unwrap_or(""),
                "git@github.com:org/skills.git"
            );
        } else {
            panic!("modal should still be open");
        }
    }

    #[test]
    fn paste_noop_when_modal_closed() {
        let (mut a, _dir) = fixture_app();
        assert!(matches!(a.settings.remotes.modal, ModalState::Closed));
        handle_paste(&mut a.settings.remotes, "should-not-appear");
    }

    // ── Connection test tests ──────────────────────────────────────────────────

    #[test]
    fn t_keybind_queues_blocking_action() {
        let (mut a, _dir) = fixture_app();
        a.settings.remotes.list_state.select(Some(0));
        handle_key(&mut a, KeyCode::Char('t'));
        assert_eq!(a.settings.remotes.testing_idx, Some(0));
        assert!(
            matches!(
                a.next_blocking,
                Some(BlockingAction::TestConnection { remote_idx: 0, .. })
            ),
            "expected TestConnection action queued"
        );
    }

    #[test]
    fn render_shows_spinner_during_testing() {
        let (mut a, _dir) = fixture_app();
        a.settings.remotes.testing_idx = Some(0);
        let mut term = Terminal::new(TestBackend::new(80, 20)).unwrap();
        term.draw(|f| crate::tui::draw(f, &a)).unwrap();
        let dump: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        let spinner_frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
        assert!(
            spinner_frames.iter().any(|g| dump.contains(g)),
            "expected a spinner glyph in output; dump: {}",
            dump
        );
    }

    #[test]
    fn render_shows_check_after_ok_result() {
        let (mut a, _dir) = fixture_app();
        a.settings.remotes.last_results.insert(
            0,
            ConnectionStatus::Ok {
                registry_size_bytes: 4096,
            },
        );
        let mut term = Terminal::new(TestBackend::new(80, 20)).unwrap();
        term.draw(|f| crate::tui::draw(f, &a)).unwrap();
        let dump: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(dump.contains('✓'), "expected ✓ in output; dump: {}", dump);
    }

    #[test]
    fn render_shows_x_after_auth_failure() {
        let (mut a, _dir) = fixture_app();
        a.settings
            .remotes
            .last_results
            .insert(0, ConnectionStatus::AuthFailed("denied".into()));
        let mut term = Terminal::new(TestBackend::new(80, 20)).unwrap();
        term.draw(|f| crate::tui::draw(f, &a)).unwrap();
        let dump: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(dump.contains('✗'), "expected ✗ in output; dump: {}", dump);
    }

    // ── Modal save / provider tests ────────────────────────────────────────────

    #[test]
    fn modal_save_persists_provider_field() {
        let (mut a, dir) = fixture_app();
        // Open add modal.
        handle_key(&mut a, KeyCode::Char('a'));

        // Type name "hub2".
        for c in "hub2".chars() {
            handle_key(&mut a, KeyCode::Char(c));
        }
        // Tab to url.
        handle_key(&mut a, KeyCode::Tab);
        for c in "https://gitlab.com/o/r.git".chars() {
            handle_key(&mut a, KeyCode::Char(c));
        }
        // Tab to provider select.
        handle_key(&mut a, KeyCode::Tab);
        // Down opens the dropdown.
        handle_key(&mut a, KeyCode::Down);
        // Down × 3 moves highlight: auto(0) → github(1) → githubenterprise(2) → gitlab(3).
        handle_key(&mut a, KeyCode::Down);
        handle_key(&mut a, KeyCode::Down);
        handle_key(&mut a, KeyCode::Down);
        // Enter selects the highlighted option (gitlab).
        handle_key(&mut a, KeyCode::Enter);
        // Verify provider via form.to_json.
        if let ModalState::AddOrEdit { form, .. } = &a.settings.remotes.modal {
            let json = form.to_json();
            assert_eq!(
                json["provider"].as_str().unwrap_or(""),
                "gitlab",
                "provider should be 'gitlab' after selection"
            );
        } else {
            panic!("modal should still be open");
        }
        // Tab through push_mode, direct_branch, checkbox, then to Submit.
        handle_key(&mut a, KeyCode::Tab);
        handle_key(&mut a, KeyCode::Tab);
        handle_key(&mut a, KeyCode::Tab);
        handle_key(&mut a, KeyCode::Tab);
        // Enter to submit.
        handle_key(&mut a, KeyCode::Enter);

        let written = std::fs::read_to_string(dir.child("user.toml").path()).unwrap();
        assert!(
            written.contains("provider = \"gitlab\""),
            "expected provider field in written TOML; content: {}",
            written
        );
    }

    #[test]
    fn modal_edit_loads_existing_provider() {
        // Build form for an existing remote with Bitbucket provider.
        let remote = RemoteConfig {
            url: "https://bitbucket.org/x/y.git".into(),
            default: false,
            provider: Some(ProviderKind::Bitbucket),
            push_mode: quay_core::PushMode::default(),
            direct_branch: None,
        };
        let form = build_remote_modal_form(Some(&remote), Some("my-remote"));
        let json = form.to_json();
        assert_eq!(
            json["provider"].as_str().unwrap_or(""),
            "bitbucket",
            "form should be pre-populated with 'bitbucket'"
        );
    }

    #[test]
    fn renders_remote_with_default_marker() {
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
        assert!(dump.contains("primary"));
        assert!(dump.contains("[default]"), "dump: {}", dump);
    }

    #[test]
    fn add_form_appends_remote_to_active_profile() {
        let (mut a, dir) = fixture_app();
        handle_key(&mut a, KeyCode::Char('a'));
        for c in "secondary".chars() {
            handle_key(&mut a, KeyCode::Char(c));
        }
        handle_key(&mut a, KeyCode::Tab); // → url
        for c in "https://x".chars() {
            handle_key(&mut a, KeyCode::Char(c));
        }
        // Tab through provider, push_mode, direct_branch, default checkbox, then Submit.
        handle_key(&mut a, KeyCode::Tab);
        handle_key(&mut a, KeyCode::Tab);
        handle_key(&mut a, KeyCode::Tab);
        handle_key(&mut a, KeyCode::Tab);
        handle_key(&mut a, KeyCode::Tab);
        handle_key(&mut a, KeyCode::Enter);
        let written = std::fs::read_to_string(dir.child("user.toml").path()).unwrap();
        assert!(written.contains("[profiles.work.remotes.secondary]"));
    }
}
