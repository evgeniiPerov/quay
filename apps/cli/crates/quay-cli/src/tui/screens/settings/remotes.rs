//! Settings → Remotes tab. Edits the active profile's remotes inside the user
//! config file.

use crate::config_io::{read_user_file, write_user_file};
use crate::tui::app::{App, ScreenAction};
use crate::tui::theme;
use crossterm::event::KeyCode;
use quay_core::RemoteConfig;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

#[derive(Debug, Default)]
pub struct RemotesState {
    pub list_state: ListState,
    pub mode: Mode,
    pub form: Form,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Browsing,
    Adding,
    ConfirmingDelete {
        name: String,
    },
    TestStub {
        name: String,
    },
}

#[derive(Debug, Default)]
pub struct Form {
    pub name: String,
    pub url: String,
    pub focused: FormField,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum FormField {
    #[default]
    Name,
    Url,
}

pub fn handle_key(app: &mut App, code: KeyCode) -> ScreenAction {
    match app.settings.remotes.mode {
        Mode::Browsing => handle_browsing(app, code),
        Mode::Adding => handle_form(app, code),
        Mode::ConfirmingDelete { .. } => handle_confirm(app, code),
        Mode::TestStub { .. } => handle_test_stub(app, code),
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
            app.settings.remotes.form = Form::default();
            app.settings.remotes.mode = Mode::Adding;
        }
        KeyCode::Char('d') => {
            if let Some(name) = selected_name(app) {
                app.settings.remotes.mode = Mode::ConfirmingDelete { name };
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
            if let Some(name) = selected_name(app) {
                app.settings.remotes.mode = Mode::TestStub { name };
            }
        }
        _ => {}
    }
    ScreenAction::Stay
}

fn handle_form(app: &mut App, code: KeyCode) -> ScreenAction {
    match code {
        KeyCode::Esc => {
            app.settings.remotes.mode = Mode::Browsing;
        }
        KeyCode::Tab => {
            app.settings.remotes.form.focused = match app.settings.remotes.form.focused {
                FormField::Name => FormField::Url,
                FormField::Url => FormField::Name,
            };
        }
        KeyCode::Enter => match submit_add(app) {
            Ok(_) => {
                app.set_status("remote added");
                app.settings.remotes.mode = Mode::Browsing;
            }
            Err(e) => app.set_status(format!("error: {}", e)),
        },
        KeyCode::Backspace => {
            let f = &mut app.settings.remotes.form;
            match f.focused {
                FormField::Name => {
                    f.name.pop();
                }
                FormField::Url => {
                    f.url.pop();
                }
            }
        }
        KeyCode::Char(c) => {
            let f = &mut app.settings.remotes.form;
            match f.focused {
                FormField::Name => f.name.push(c),
                FormField::Url => f.url.push(c),
            }
        }
        _ => {}
    }
    ScreenAction::Stay
}

fn handle_confirm(app: &mut App, code: KeyCode) -> ScreenAction {
    let name = match &app.settings.remotes.mode {
        Mode::ConfirmingDelete { name } => name.clone(),
        _ => return ScreenAction::Stay,
    };
    match code {
        KeyCode::Char('y') | KeyCode::Enter => match submit_delete(app, &name) {
            Ok(_) => {
                app.set_status(format!("removed remote '{}'", name));
                app.settings.remotes.mode = Mode::Browsing;
            }
            Err(e) => {
                app.set_status(format!("error: {}", e));
                app.settings.remotes.mode = Mode::Browsing;
            }
        },
        KeyCode::Esc | KeyCode::Char('n') => {
            app.settings.remotes.mode = Mode::Browsing;
        }
        _ => {}
    }
    ScreenAction::Stay
}

fn handle_test_stub(app: &mut App, code: KeyCode) -> ScreenAction {
    if matches!(code, KeyCode::Esc | KeyCode::Enter | KeyCode::Char(_)) {
        app.settings.remotes.mode = Mode::Browsing;
    }
    ScreenAction::Stay
}

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

fn submit_add(app: &mut App) -> Result<(), Box<dyn std::error::Error>> {
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
    let name = app.settings.remotes.form.name.trim().to_string();
    let url = app.settings.remotes.form.url.trim().to_string();
    if name.is_empty() || url.is_empty() {
        return Err("name and url required".into());
    }
    if p.remotes.contains_key(&name) {
        return Err(format!("remote '{}' already exists", name).into());
    }
    p.remotes.insert(
        name,
        RemoteConfig {
            url,
            default: p.remotes.is_empty(),
            provider: None,
        },
    );
    write_user_file(path, &file)?;
    Ok(())
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
    let items: Vec<ListItem> = active
        .as_deref()
        .and_then(|a| file.profiles.get(a))
        .map(|p| {
            p.remotes
                .iter()
                .map(|(name, r)| {
                    let tag = if r.default { " [default]" } else { "" };
                    ListItem::new(Line::from(format!("{}\t{}{}", name, r.url, tag)))
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
        Line::from("[a] add  [d] delete"),
        Line::from("[s] set default  [t] test"),
        Line::from("[Tab] next tab  [q] quit"),
    ];
    frame.render_widget(
        Paragraph::new(hints).block(Block::default().borders(Borders::ALL).title(" Actions ")),
        cols[1],
    );

    match &app.settings.remotes.mode {
        Mode::Adding => render_form_modal(frame, area, "Add remote", &app.settings.remotes.form),
        Mode::ConfirmingDelete { name } => render_confirm_modal(frame, area, name),
        Mode::TestStub { name } => render_test_stub(frame, area, name),
        Mode::Browsing => {}
    }
}

fn render_form_modal(frame: &mut Frame, area: Rect, title: &str, form: &Form) {
    let modal_area = centered_rect(area, 60, 30);
    frame.render_widget(Clear, modal_area);
    let block = Block::default().borders(Borders::ALL).title(title);
    let inner = block.inner(modal_area);
    frame.render_widget(block, modal_area);

    let lines = vec![
        Line::from(format!(
            "{} name: {}",
            if form.focused == FormField::Name {
                "▶"
            } else {
                " "
            },
            form.name
        )),
        Line::from(format!(
            "{} url:  {}",
            if form.focused == FormField::Url {
                "▶"
            } else {
                " "
            },
            form.url
        )),
        Line::from(""),
        Line::from("Tab — switch field   Enter — save   Esc — cancel"),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
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

fn render_test_stub(frame: &mut Frame, area: Rect, name: &str) {
    let modal_area = centered_rect(area, 50, 25);
    frame.render_widget(Clear, modal_area);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Test connection ");
    let inner = block.inner(modal_area);
    frame.render_widget(block, modal_area);
    let lines = vec![
        Line::from(format!("Stub: would test '{}' here.", name)),
        Line::from(""),
        Line::from("Real network probing lands in Plan 7."),
        Line::from(""),
        Line::from("Press any key to dismiss."),
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

#[cfg(test)]
mod tests {
    use super::*;
    use assert_fs::prelude::*;
    use quay_core::{Config, Lockfile, ProfileFile};
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
            },
        );
        file.profiles.insert("work".into(), p);
        write_user_file(user_path.path(), &file).unwrap();

        let mut a = App::new(
            Config::default(),
            Lockfile::default(),
            dir.path().to_path_buf(),
            Some(user_path.path().to_path_buf()),
        );
        a.current_screen = crate::tui::app::Screen::Settings;
        a.settings.tab = crate::tui::app::SettingsTab::Remotes;
        (a, dir)
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
        handle_key(&mut a, KeyCode::Tab);
        for c in "https://x".chars() {
            handle_key(&mut a, KeyCode::Char(c));
        }
        handle_key(&mut a, KeyCode::Enter);
        let written = std::fs::read_to_string(dir.child("user.toml").path()).unwrap();
        assert!(written.contains("[profiles.work.remotes.secondary]"));
    }
}
