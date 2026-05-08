use crate::tui::app::{App, Screen, ScreenAction};
use crate::tui::theme;
use crossterm::event::KeyCode;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use ratatui::Frame;

pub fn handle_key(_app: &mut App, code: KeyCode) -> ScreenAction {
    match code {
        KeyCode::Char('b') => ScreenAction::SwitchTo(Screen::Browse),
        KeyCode::Char('s') => ScreenAction::SwitchTo(Screen::Search),
        KeyCode::Char('i') => ScreenAction::SwitchTo(Screen::Installed),
        KeyCode::Char('c') => ScreenAction::SwitchTo(Screen::CreatePush),
        _ => ScreenAction::Stay,
    }
}

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let outer = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(" quay ", theme::accent()));
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(8),
            Constraint::Length(2),
        ])
        .split(inner);

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Dashboard", theme::accent()),
            Span::raw("    "),
            Span::styled(
                "press [b] browse  [s] search  [i] installed  [c] create  [q] quit",
                theme::dim(),
            ),
        ])),
        rows[0],
    );

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(33),
            Constraint::Percentage(33),
            Constraint::Percentage(34),
        ])
        .split(rows[1]);

    let remote_items: Vec<ListItem> = app
        .cfg
        .remotes
        .iter()
        .map(|(name, r)| {
            let marker = if r.default { "★ " } else { "  " };
            ListItem::new(Line::from(format!("{}{}", marker, name)))
        })
        .collect();
    frame.render_widget(
        List::new(remote_items).block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" Remotes ({}) ", app.cfg.remotes.len())),
        ),
        cols[0],
    );

    let installed_items: Vec<ListItem> = app
        .lock
        .skills
        .iter()
        .map(|(name, s)| ListItem::new(Line::from(format!("{} v{}", name, s.version))))
        .collect();
    frame.render_widget(
        List::new(installed_items).block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" Installed ({}) ", app.lock.skills.len())),
        ),
        cols[1],
    );

    frame.render_widget(
        List::new(vec![ListItem::new(Line::from(Span::styled(
            "(run `quay outdated` from CLI)",
            theme::dim(),
        )))])
        .block(Block::default().borders(Borders::ALL).title(" Outdated ")),
        cols[2],
    );

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "Tip: [1]/[2]/[3]/[4] jump to screen; [c] create new skill; [q] quit",
            theme::dim(),
        ))),
        rows[2],
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use quay_core::{Config, Lockfile, RemoteConfig};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn fixture_app() -> App {
        let mut cfg = Config::default();
        cfg.remotes.insert(
            "primary".into(),
            RemoteConfig {
                url: "https://x".into(),
                default: true,
                provider: None,
            },
        );
        cfg.user.email = Some("dev@example.com".into());
        let lock = Lockfile::default();
        App::new(cfg, lock, std::path::PathBuf::from("/tmp"), None)
    }

    #[test]
    fn dashboard_shows_remote_and_count_titles() {
        let mut terminal = Terminal::new(TestBackend::new(80, 20)).unwrap();
        let app = fixture_app();
        terminal.draw(|f| crate::tui::draw(f, &app)).unwrap();
        let dump: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(dump.contains("Remotes (1)"), "buffer: {}", dump);
        assert!(dump.contains("Installed (0)"));
        assert!(dump.contains("Dashboard"));
    }

    #[test]
    fn dashboard_b_key_switches_to_browse() {
        let mut app = fixture_app();
        let action = handle_key(&mut app, KeyCode::Char('b'));
        match action {
            ScreenAction::SwitchTo(Screen::Browse) => {}
            _ => panic!("expected SwitchTo(Browse)"),
        }
    }

    #[test]
    fn dashboard_c_key_switches_to_create_push() {
        let mut app = fixture_app();
        let action = handle_key(&mut app, KeyCode::Char('c'));
        match action {
            ScreenAction::SwitchTo(Screen::CreatePush) => {}
            _ => panic!("expected SwitchTo(CreatePush), got {:?}", action),
        }
    }
}
