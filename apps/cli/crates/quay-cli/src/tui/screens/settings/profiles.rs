//! Settings → Profiles tab.

use crate::config_io::{read_user_file, write_user_file};
use crate::tui::app::{App, ScreenAction};
use crate::tui::theme;
use crossterm::event::KeyCode;
use quay_core::ProfileFile;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

#[derive(Debug, Default)]
pub struct ProfilesState {
    pub list_state: ListState,
    pub mode: Mode,
    pub form: Form,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Browsing,
    Adding,
    Renaming {
        old: String,
    },
    ConfirmingDelete {
        name: String,
    },
}

#[derive(Debug, Default)]
pub struct Form {
    pub name: String,
    pub email: String,
    pub focused: FormField,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum FormField {
    #[default]
    Name,
    Email,
}

pub fn handle_key(app: &mut App, code: KeyCode) -> ScreenAction {
    match app.settings.profiles.mode {
        Mode::Browsing => handle_browsing(app, code),
        Mode::Adding => handle_form(app, code, FormPurpose::Add),
        Mode::Renaming { .. } => handle_form(app, code, FormPurpose::Rename),
        Mode::ConfirmingDelete { .. } => handle_confirm(app, code),
    }
}

#[derive(Clone, Copy)]
enum FormPurpose {
    Add,
    Rename,
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
            app.settings.profiles.form = Form::default();
            app.settings.profiles.mode = Mode::Adding;
        }
        KeyCode::Char('r') => {
            if let Some(name) = selected_name(app) {
                app.settings.profiles.form = Form {
                    name: name.clone(),
                    email: String::new(),
                    focused: FormField::Name,
                };
                app.settings.profiles.mode = Mode::Renaming { old: name };
            }
        }
        KeyCode::Char('d') => {
            if let Some(name) = selected_name(app) {
                app.settings.profiles.mode = Mode::ConfirmingDelete { name };
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

fn handle_form(app: &mut App, code: KeyCode, purpose: FormPurpose) -> ScreenAction {
    match code {
        KeyCode::Esc => {
            app.settings.profiles.mode = Mode::Browsing;
        }
        KeyCode::Tab => {
            app.settings.profiles.form.focused = match app.settings.profiles.form.focused {
                FormField::Name => FormField::Email,
                FormField::Email => FormField::Name,
            };
        }
        KeyCode::Enter => match purpose {
            FormPurpose::Add => match submit_add(app) {
                Ok(_) => {
                    app.set_status("profile added");
                    app.settings.profiles.mode = Mode::Browsing;
                }
                Err(e) => app.set_status(format!("error: {}", e)),
            },
            FormPurpose::Rename => match submit_rename(app) {
                Ok(_) => {
                    app.set_status("profile renamed");
                    app.settings.profiles.mode = Mode::Browsing;
                }
                Err(e) => app.set_status(format!("error: {}", e)),
            },
        },
        KeyCode::Backspace => {
            let f = &mut app.settings.profiles.form;
            match f.focused {
                FormField::Name => {
                    f.name.pop();
                }
                FormField::Email => {
                    f.email.pop();
                }
            }
        }
        KeyCode::Char(c) => {
            let f = &mut app.settings.profiles.form;
            match f.focused {
                FormField::Name => f.name.push(c),
                FormField::Email => f.email.push(c),
            }
        }
        _ => {}
    }
    ScreenAction::Stay
}

fn handle_confirm(app: &mut App, code: KeyCode) -> ScreenAction {
    let name = match &app.settings.profiles.mode {
        Mode::ConfirmingDelete { name } => name.clone(),
        _ => return ScreenAction::Stay,
    };
    match code {
        KeyCode::Char('y') | KeyCode::Enter => match submit_delete(app, &name) {
            Ok(_) => {
                app.set_status(format!("removed profile '{}'", name));
                app.settings.profiles.mode = Mode::Browsing;
            }
            Err(e) => {
                app.set_status(format!("error: {}", e));
                app.settings.profiles.mode = Mode::Browsing;
            }
        },
        KeyCode::Esc | KeyCode::Char('n') => {
            app.settings.profiles.mode = Mode::Browsing;
        }
        _ => {}
    }
    ScreenAction::Stay
}

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

fn submit_add(app: &mut App) -> Result<(), Box<dyn std::error::Error>> {
    let path = app
        .user_config_path
        .as_deref()
        .ok_or("--user-config required")?;
    let mut file = read_user_file(Some(path))?;
    let name = app.settings.profiles.form.name.trim().to_string();
    if name.is_empty() {
        return Err("name required".into());
    }
    if file.profiles.contains_key(&name) {
        return Err(format!("profile '{}' already exists", name).into());
    }
    let mut p = ProfileFile::default();
    let email = app.settings.profiles.form.email.trim();
    if !email.is_empty() {
        p.user.email = Some(email.into());
    }
    file.profiles.insert(name.clone(), p);
    if file.active_profile.is_none() {
        file.active_profile = Some(name);
    }
    write_user_file(path, &file)?;
    Ok(())
}

fn submit_rename(app: &mut App) -> Result<(), Box<dyn std::error::Error>> {
    let path = app
        .user_config_path
        .as_deref()
        .ok_or("--user-config required")?;
    let mut file = read_user_file(Some(path))?;
    let new_name = app.settings.profiles.form.name.trim().to_string();
    let old = match &app.settings.profiles.mode {
        Mode::Renaming { old } => old.clone(),
        _ => return Err("not in rename mode".into()),
    };
    if new_name.is_empty() {
        return Err("name required".into());
    }
    if file.profiles.contains_key(&new_name) && new_name != old {
        return Err(format!("profile '{}' already exists", new_name).into());
    }
    let p = file
        .profiles
        .remove(&old)
        .ok_or(format!("profile '{}' missing", old))?;
    file.profiles.insert(new_name.clone(), p);
    if file.active_profile.as_deref() == Some(old.as_str()) {
        file.active_profile = Some(new_name);
    }
    write_user_file(path, &file)?;
    Ok(())
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

    // Modal overlay for forms.
    match &app.settings.profiles.mode {
        Mode::Adding => render_form_modal(frame, area, "Add profile", &app.settings.profiles.form),
        Mode::Renaming { old } => render_form_modal(
            frame,
            area,
            &format!("Rename '{}'", old),
            &app.settings.profiles.form,
        ),
        Mode::ConfirmingDelete { name } => render_confirm_modal(frame, area, name),
        Mode::Browsing => {}
    }
}

fn render_form_modal(frame: &mut Frame, area: Rect, title: &str, form: &Form) {
    let modal_area = centered_rect(area, 50, 30);
    frame.render_widget(Clear, modal_area);
    let block = Block::default().borders(Borders::ALL).title(title);
    let inner = block.inner(modal_area);
    frame.render_widget(block, modal_area);

    let lines = vec![
        Line::from(format!(
            "{} name:  {}",
            if form.focused == FormField::Name {
                "▶"
            } else {
                " "
            },
            form.name
        )),
        Line::from(format!(
            "{} email: {}",
            if form.focused == FormField::Email {
                "▶"
            } else {
                " "
            },
            form.email
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
        assert!(matches!(a.settings.profiles.mode, Mode::Adding));
        // Type "personal" into name field.
        for c in "personal".chars() {
            handle_key(&mut a, KeyCode::Char(c));
        }
        // Tab to email.
        handle_key(&mut a, KeyCode::Tab);
        for c in "e@home".chars() {
            handle_key(&mut a, KeyCode::Char(c));
        }
        // Submit.
        handle_key(&mut a, KeyCode::Enter);
        // Assert disk.
        let written = std::fs::read_to_string(dir.child("user.toml").path()).unwrap();
        assert!(written.contains("[profiles.personal"), "wrote: {}", written);
        assert!(written.contains("e@home"));
        assert!(matches!(a.settings.profiles.mode, Mode::Browsing));
    }

    #[test]
    fn delete_confirm_removes_profile() {
        let (mut a, dir) = fixture_app();
        // Pre-add a second profile so deletion is allowed.
        let mut file = read_user_file(a.user_config_path.as_deref()).unwrap();
        file.profiles
            .insert("personal".into(), ProfileFile::default());
        write_user_file(a.user_config_path.as_deref().unwrap(), &file).unwrap();

        // Move selection to second item ("work" sorts first because BTreeMap; "personal" is index 0).
        // BTreeMap orders alphabetically: personal, work. Select index 1 (work).
        handle_key(&mut a, KeyCode::Down);
        // Press d to enter confirm mode.
        handle_key(&mut a, KeyCode::Char('d'));
        assert!(matches!(
            a.settings.profiles.mode,
            Mode::ConfirmingDelete { .. }
        ));
        handle_key(&mut a, KeyCode::Char('y'));
        // Disk should no longer contain [profiles.work
        let written = std::fs::read_to_string(dir.child("user.toml").path()).unwrap();
        assert!(!written.contains("[profiles.work"), "wrote: {}", written);
    }
}
