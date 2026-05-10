//! Installed-skills screen — shows locally-found skills from the scanner.
//!
//! Replaces the old lockfile-based view. Data comes from `app.local_skills`.

use crate::tui::app::{App, ScreenAction};
use crate::tui::theme;
use crossterm::event::KeyCode;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

#[derive(Debug, Default)]
pub struct InstalledState {
    pub list_state: ListState,
    pub outdated_only: bool,
}

pub fn handle_key(app: &mut App, code: KeyCode) -> ScreenAction {
    match code {
        KeyCode::Down | KeyCode::Char('j') => {
            let i = app.installed.list_state.selected().unwrap_or(0);
            app.installed.list_state.select(Some(i.saturating_add(1)));
        }
        KeyCode::Up | KeyCode::Char('k') => {
            let i = app.installed.list_state.selected().unwrap_or(0);
            app.installed.list_state.select(Some(i.saturating_sub(1)));
        }
        KeyCode::Char('o') => {
            app.installed.outdated_only = !app.installed.outdated_only;
        }
        _ => {}
    }
    ScreenAction::Stay
}

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    let skills = &app.local_skills;
    let items: Vec<ListItem> = skills
        .iter()
        .map(|s| {
            let mirrors: Vec<&str> = s.locations.iter().map(|l| l.root.label()).collect();
            ListItem::new(Line::from(format!(
                "{}  v{}  [{}]",
                s.meta.name,
                s.meta.version,
                mirrors.join(",")
            )))
        })
        .collect();
    let title = format!(" Local Skills ({}) ", skills.len());
    let mut list_state = app.installed.list_state.clone();
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(title))
        .highlight_style(theme::selected());
    frame.render_stateful_widget(list, cols[0], &mut list_state);

    let preview = app
        .installed
        .list_state
        .selected()
        .and_then(|i| skills.get(i))
        .map(|s| {
            let mirrors: Vec<&str> = s.locations.iter().map(|l| l.root.label()).collect();
            format!(
                "{} v{}\nmirrors: {}\npath: {}\n",
                s.meta.name,
                s.meta.version,
                mirrors.join(", "),
                s.canonical_path().display(),
            )
        })
        .unwrap_or_else(|| "(no local skills found)".into());
    frame.render_widget(
        Paragraph::new(preview).block(Block::default().borders(Borders::ALL).title(" Detail ")),
        cols[1],
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use quay_core::Config;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn fixture_app() -> App {
        App::new(Config::default(), std::path::PathBuf::from("/tmp"), None)
    }

    #[test]
    fn installed_renders_without_crash() {
        let mut terminal = Terminal::new(TestBackend::new(80, 20)).unwrap();
        let mut a = fixture_app();
        a.current_screen = crate::tui::app::Screen::Installed;
        terminal.draw(|f| crate::tui::draw(f, &a)).unwrap();
        // Just verify it doesn't panic.
    }

    #[test]
    fn installed_outdated_toggle_flips_state() {
        let mut a = fixture_app();
        let _ = handle_key(&mut a, KeyCode::Char('o'));
        assert!(a.installed.outdated_only);
        let _ = handle_key(&mut a, KeyCode::Char('o'));
        assert!(!a.installed.outdated_only);
    }
}
