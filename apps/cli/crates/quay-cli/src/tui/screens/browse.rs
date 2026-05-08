use crate::tui::app::{App, ScreenAction};
use crate::tui::theme;
use crossterm::event::KeyCode;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

#[derive(Debug, Default)]
pub struct BrowseState {
    pub items: Vec<BrowseRow>,
    pub list_state: ListState,
}

#[derive(Debug, Clone)]
pub enum BrowseRow {
    Remote { name: String },
}

pub fn handle_key(app: &mut App, code: KeyCode) -> ScreenAction {
    match code {
        KeyCode::Down | KeyCode::Char('j') => {
            let max = app.browse.items.len().saturating_sub(1);
            let i = app.browse.list_state.selected().unwrap_or(0);
            app.browse
                .list_state
                .select(Some(i.saturating_add(1).min(max)));
        }
        KeyCode::Up | KeyCode::Char('k') => {
            let i = app.browse.list_state.selected().unwrap_or(0);
            app.browse.list_state.select(Some(i.saturating_sub(1)));
        }
        _ => {}
    }
    ScreenAction::Stay
}

fn ensure_loaded(app: &App, state: &mut BrowseState) {
    if !state.items.is_empty() {
        return;
    }
    for rname in app.cfg.remotes.keys() {
        state.items.push(BrowseRow::Remote {
            name: rname.clone(),
        });
    }
    if !state.items.is_empty() {
        state.list_state.select(Some(0));
    }
}

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    // We need to mutate browse state on first render. Use a local clone for population
    // because the function takes &App. Workaround: clone the items list into a local.
    let mut local = if app.browse.items.is_empty() {
        let mut tmp = BrowseState::default();
        ensure_loaded(app, &mut tmp);
        tmp
    } else {
        BrowseState {
            items: app.browse.items.clone(),
            list_state: app.browse.list_state.clone(),
        }
    };

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(area);

    let items: Vec<ListItem> = local
        .items
        .iter()
        .map(|row| match row {
            BrowseRow::Remote { name } => ListItem::new(Line::from(vec![
                Span::styled("● ", theme::accent()),
                Span::raw(name.clone()),
            ])),
        })
        .collect();

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(" Browse "))
        .highlight_style(theme::selected());
    frame.render_stateful_widget(list, cols[0], &mut local.list_state);

    frame.render_widget(
        Paragraph::new("(switch to Search to fetch skills from configured remotes)")
            .block(Block::default().borders(Borders::ALL).title(" Preview ")),
        cols[1],
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
            "team-hub".into(),
            RemoteConfig {
                url: "https://x".into(),
                default: true,
                provider: None,
            },
        );
        let mut a = App::new(
            cfg,
            Lockfile::default(),
            std::path::PathBuf::from("/tmp"),
            None,
        );
        a.current_screen = crate::tui::app::Screen::Browse;
        a
    }

    #[test]
    fn browse_renders_remote_name() {
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
        assert!(dump.contains("team-hub"), "dump: {}", dump);
    }
}
