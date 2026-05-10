//! Settings → Install tab. Edits the project config's `[install].mirrors` and
//! offers a check button mapped to the same logic as `quay link check`.

use crate::config_io::{read_project_file, write_project_file};
use crate::tui::app::{App, ScreenAction};
use crate::tui::theme;
use crossterm::event::KeyCode;
use quay_core::{check, MirrorConfig, MirrorStrategy};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

#[derive(Debug, Default)]
pub struct InstallState {
    pub list_state: ListState,
    pub mode: Mode,
    pub form: Form,
    pub last_check: Option<String>,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Browsing,
    Adding,
    ConfirmingDelete {
        path: String,
    },
}

#[derive(Debug, Default)]
pub struct Form {
    pub path: String,
    pub strategy: String,
    pub focused: FormField,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum FormField {
    #[default]
    Path,
    Strategy,
}

/// Insert a pasted string into the focused text field of the add modal.
///
/// Silently dropped if the modal is not open.
pub fn handle_paste(state: &mut InstallState, s: &str) {
    let safe: String = s.chars().filter(|c| *c != '\r' && *c != '\n').collect();
    if state.mode == Mode::Adding {
        match state.form.focused {
            FormField::Path => state.form.path.push_str(&safe),
            FormField::Strategy => state.form.strategy.push_str(&safe),
        }
    }
}

pub fn handle_key(app: &mut App, code: KeyCode) -> ScreenAction {
    match app.settings.install.mode {
        Mode::Browsing => handle_browsing(app, code),
        Mode::Adding => handle_form(app, code),
        Mode::ConfirmingDelete { .. } => handle_confirm(app, code),
    }
}

fn handle_browsing(app: &mut App, code: KeyCode) -> ScreenAction {
    match code {
        KeyCode::Down | KeyCode::Char('j') => {
            let max = mirror_count(app).saturating_sub(1);
            let i = app.settings.install.list_state.selected().unwrap_or(0);
            app.settings
                .install
                .list_state
                .select(Some(i.saturating_add(1).min(max)));
        }
        KeyCode::Up => {
            let i = app.settings.install.list_state.selected().unwrap_or(0);
            app.settings
                .install
                .list_state
                .select(Some(i.saturating_sub(1)));
        }
        KeyCode::Char('a') => {
            app.settings.install.form = Form {
                strategy: "auto".into(),
                ..Default::default()
            };
            app.settings.install.mode = Mode::Adding;
        }
        KeyCode::Char('d') => {
            if let Some(path) = selected_path(app) {
                app.settings.install.mode = Mode::ConfirmingDelete { path };
            }
        }
        KeyCode::Char('k') => {
            // Run `quay link check` equivalent.
            let result = run_check(app);
            app.settings.install.last_check = Some(result);
        }
        _ => {}
    }
    ScreenAction::Stay
}

fn handle_form(app: &mut App, code: KeyCode) -> ScreenAction {
    match code {
        KeyCode::Esc => {
            app.settings.install.mode = Mode::Browsing;
        }
        KeyCode::Tab => {
            app.settings.install.form.focused = match app.settings.install.form.focused {
                FormField::Path => FormField::Strategy,
                FormField::Strategy => FormField::Path,
            };
        }
        KeyCode::Enter => match submit_add(app) {
            Ok(_) => {
                app.set_status("mirror added");
                app.settings.install.mode = Mode::Browsing;
            }
            Err(e) => app.set_status(format!("error: {}", e)),
        },
        KeyCode::Backspace => {
            let f = &mut app.settings.install.form;
            match f.focused {
                FormField::Path => {
                    f.path.pop();
                }
                FormField::Strategy => {
                    f.strategy.pop();
                }
            }
        }
        KeyCode::Char(c) => {
            let f = &mut app.settings.install.form;
            match f.focused {
                FormField::Path => f.path.push(c),
                FormField::Strategy => f.strategy.push(c),
            }
        }
        _ => {}
    }
    ScreenAction::Stay
}

fn handle_confirm(app: &mut App, code: KeyCode) -> ScreenAction {
    let path = match &app.settings.install.mode {
        Mode::ConfirmingDelete { path } => path.clone(),
        _ => return ScreenAction::Stay,
    };
    match code {
        KeyCode::Char('y') | KeyCode::Enter => match submit_delete(app, &path) {
            Ok(_) => {
                app.set_status(format!("removed mirror '{}'", path));
                app.settings.install.mode = Mode::Browsing;
            }
            Err(e) => {
                app.set_status(format!("error: {}", e));
                app.settings.install.mode = Mode::Browsing;
            }
        },
        KeyCode::Esc | KeyCode::Char('n') => {
            app.settings.install.mode = Mode::Browsing;
        }
        _ => {}
    }
    ScreenAction::Stay
}

fn mirror_count(app: &App) -> usize {
    read_project_file(&app.project_root)
        .map(|f| f.install.mirrors.len())
        .unwrap_or(0)
}

fn selected_path(app: &App) -> Option<String> {
    let f = read_project_file(&app.project_root).ok()?;
    let i = app.settings.install.list_state.selected().unwrap_or(0);
    f.install
        .mirrors
        .get(i)
        .map(|m| m.path.display().to_string())
}

fn submit_add(app: &mut App) -> Result<(), Box<dyn std::error::Error>> {
    let mut file = read_project_file(&app.project_root)?;
    let path = app.settings.install.form.path.trim();
    if path.is_empty() {
        return Err("path required".into());
    }
    if file
        .install
        .mirrors
        .iter()
        .any(|m| m.path.to_string_lossy() == path)
    {
        return Err(format!("mirror at '{}' already configured", path).into());
    }
    let strategy = match app.settings.install.form.strategy.trim() {
        "auto" | "" => MirrorStrategy::Auto,
        "symlink" => MirrorStrategy::Symlink,
        "junction" => MirrorStrategy::Junction,
        "copy" => MirrorStrategy::Copy,
        other => return Err(format!("invalid strategy: {}", other).into()),
    };
    file.install.mirrors.push(MirrorConfig {
        path: path.into(),
        strategy,
    });
    write_project_file(&app.project_root, &file)?;
    Ok(())
}

fn submit_delete(app: &mut App, path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut file = read_project_file(&app.project_root)?;
    let before = file.install.mirrors.len();
    file.install
        .mirrors
        .retain(|m| m.path.to_string_lossy() != path);
    if file.install.mirrors.len() == before {
        return Err(format!("mirror '{}' not found", path).into());
    }
    write_project_file(&app.project_root, &file)?;
    Ok(())
}

fn run_check(app: &App) -> String {
    let project_config = app.project_root.join(".quay/config.toml");
    let cfg = match quay_core::Config::load_resolved(
        app.user_config_path.as_deref(),
        Some(&project_config),
        None,
    ) {
        Ok(c) => c,
        Err(e) => return format!("error: {}", e),
    };
    let canonical = app.project_root.join(&cfg.install.canonical);
    let mut names = Vec::new();
    if canonical.exists() {
        if let Ok(entries) = std::fs::read_dir(&canonical) {
            for entry in entries.flatten() {
                if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    if let Some(n) = entry.file_name().to_str() {
                        names.push(n.to_string());
                    }
                }
            }
        }
    }
    match check(&cfg.install, &app.project_root, &names) {
        Ok(drift) if drift.is_empty() => "ok: all mirrors intact".into(),
        Ok(drift) => format!("{} mirror(s) out of sync", drift.len()),
        Err(e) => format!("error: {}", e),
    }
}

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(area);

    let file = read_project_file(&app.project_root).unwrap_or_default();
    let items: Vec<ListItem> = file
        .install
        .mirrors
        .iter()
        .map(|m| {
            ListItem::new(Line::from(format!(
                "{}\t{:?}",
                m.path.display(),
                m.strategy
            )))
        })
        .collect();
    let title = format!(
        " Mirrors  (canonical: {}) ",
        file.install.canonical.display()
    );
    let mut list_state = app.settings.install.list_state.clone();
    frame.render_stateful_widget(
        List::new(items)
            .block(Block::default().borders(Borders::ALL).title(title))
            .highlight_style(theme::selected()),
        cols[0],
        &mut list_state,
    );

    let mut hint_lines = vec![
        Line::from("[a] add mirror  [d] delete"),
        Line::from("[k] check (verify intact)"),
        Line::from("[Tab] next tab  [q] quit"),
    ];
    if let Some(msg) = &app.settings.install.last_check {
        hint_lines.push(Line::from(""));
        hint_lines.push(Line::from(format!("last check: {}", msg)));
    }
    frame.render_widget(
        Paragraph::new(hint_lines).block(Block::default().borders(Borders::ALL).title(" Actions ")),
        cols[1],
    );

    match &app.settings.install.mode {
        Mode::Adding => render_form_modal(frame, area, "Add mirror", &app.settings.install.form),
        Mode::ConfirmingDelete { path } => render_confirm_modal(frame, area, path),
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
            "{} path:     {}",
            if form.focused == FormField::Path {
                "▶"
            } else {
                " "
            },
            form.path
        )),
        Line::from(format!(
            "{} strategy: {}",
            if form.focused == FormField::Strategy {
                "▶"
            } else {
                " "
            },
            form.strategy
        )),
        Line::from(""),
        Line::from("(strategy: auto | symlink | junction | copy)"),
        Line::from("Tab — switch field   Enter — save   Esc — cancel"),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_confirm_modal(frame: &mut Frame, area: Rect, path: &str) {
    let modal_area = centered_rect(area, 50, 20);
    frame.render_widget(Clear, modal_area);
    let block = Block::default().borders(Borders::ALL).title(" Confirm ");
    let inner = block.inner(modal_area);
    frame.render_widget(block, modal_area);
    let lines = vec![
        Line::from(format!("Delete mirror '{}'?", path)),
        Line::from(""),
        Line::from("(directory contents are NOT deleted)"),
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
    use quay_core::{Config, ProjectConfigFile};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn fixture_app() -> (App, assert_fs::TempDir) {
        let dir = assert_fs::TempDir::new().unwrap();
        let mut file = ProjectConfigFile::default();
        file.install.mirrors.push(MirrorConfig {
            path: ".claude/skills".into(),
            strategy: MirrorStrategy::Auto,
        });
        write_project_file(dir.path(), &file).unwrap();

        let mut a = App::new(Config::default(), dir.path().to_path_buf(), None);
        a.current_screen = crate::tui::app::Screen::Settings;
        a.settings.tab = crate::tui::app::SettingsTab::Install;
        (a, dir)
    }

    #[test]
    fn renders_existing_mirror() {
        let (a, _dir) = fixture_app();
        let mut term = Terminal::new(TestBackend::new(100, 20)).unwrap();
        term.draw(|f| crate::tui::draw(f, &a)).unwrap();
        let dump: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(dump.contains(".claude/skills"), "dump: {}", dump);
        assert!(dump.contains("Auto"), "dump: {}", dump);
    }

    #[test]
    fn add_mirror_via_form() {
        let (mut a, dir) = fixture_app();
        handle_key(&mut a, KeyCode::Char('a'));
        for c in ".cursor/rules".chars() {
            handle_key(&mut a, KeyCode::Char(c));
        }
        handle_key(&mut a, KeyCode::Enter);
        let written = std::fs::read_to_string(dir.child(".quay/config.toml").path()).unwrap();
        assert!(written.contains(".cursor/rules"), "wrote: {}", written);
    }
}
