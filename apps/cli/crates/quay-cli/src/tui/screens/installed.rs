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

    let entries: Vec<(&String, &quay_core::LockedSkill)> = app.lock.skills.iter().collect();
    let items: Vec<ListItem> = entries
        .iter()
        .map(|(name, sk)| {
            ListItem::new(Line::from(format!(
                "{}  v{}  ({})",
                name, sk.version, sk.remote
            )))
        })
        .collect();
    let title = format!(" Installed ({}) ", entries.len());
    let mut list_state = app.installed.list_state.clone();
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(title))
        .highlight_style(theme::selected());
    frame.render_stateful_widget(list, cols[0], &mut list_state);

    let preview = app
        .installed
        .list_state
        .selected()
        .and_then(|i| entries.get(i))
        .map(|(name, sk)| {
            format!(
                "{} v{}\nremote: {}\nsha: {}\n",
                name, sk.version, sk.remote, sk.sha
            )
        })
        .unwrap_or_else(|| "(no installed skills)".into());
    frame.render_widget(
        Paragraph::new(preview).block(Block::default().borders(Borders::ALL).title(" Detail ")),
        cols[1],
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use quay_core::{Config, LockedSkill, Lockfile};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn fixture_app() -> App {
        let mut lock = Lockfile::default();
        lock.skills.insert(
            "csv-parse".into(),
            LockedSkill {
                remote: "primary".into(),
                version: "0.1.0".into(),
                sha: "deadbeef".into(),
                path: "skills/csv-parse".into(),
                files: vec![],
                installed_at: "2026-05-08T00:00:00Z".into(),
            },
        );
        let mut a = App::new(
            Config::default(),
            lock,
            std::path::PathBuf::from("/tmp"),
            None,
        );
        a.current_screen = crate::tui::app::Screen::Installed;
        a
    }

    #[test]
    fn installed_renders_skill_row() {
        let mut terminal = Terminal::new(TestBackend::new(80, 20)).unwrap();
        let a = fixture_app();
        terminal.draw(|f| crate::tui::draw(f, &a)).unwrap();
        let dump: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(dump.contains("csv-parse"), "dump: {}", dump);
        assert!(dump.contains("Installed (1)"), "dump: {}", dump);
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
