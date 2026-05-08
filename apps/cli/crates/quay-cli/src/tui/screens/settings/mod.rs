//! Screen 6 — Settings. Three tabs: Profiles, Remotes, Install.

pub mod install;
pub mod profiles;
pub mod remotes;

use crate::tui::app::{App, ScreenAction, SettingsTab};
use crate::tui::theme;
use crossterm::event::KeyCode;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

pub fn handle_key(app: &mut App, code: KeyCode) -> ScreenAction {
    // Tab cycling at the screen level; tabs handle remaining keys.
    match code {
        KeyCode::Tab => {
            app.settings.tab = match app.settings.tab {
                SettingsTab::Profiles => SettingsTab::Remotes,
                SettingsTab::Remotes => SettingsTab::Install,
                SettingsTab::Install => SettingsTab::Profiles,
            };
            return ScreenAction::Stay;
        }
        KeyCode::BackTab => {
            app.settings.tab = match app.settings.tab {
                SettingsTab::Profiles => SettingsTab::Install,
                SettingsTab::Remotes => SettingsTab::Profiles,
                SettingsTab::Install => SettingsTab::Remotes,
            };
            return ScreenAction::Stay;
        }
        _ => {}
    }
    match app.settings.tab {
        SettingsTab::Profiles => profiles::handle_key(app, code),
        SettingsTab::Remotes => remotes::handle_key(app, code),
        SettingsTab::Install => install::handle_key(app, code),
    }
}

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let outer = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(" Settings ", theme::accent()));
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(inner);

    frame.render_widget(Paragraph::new(tab_strip(app)), rows[0]);

    match app.settings.tab {
        SettingsTab::Profiles => profiles::render(frame, app, rows[1]),
        SettingsTab::Remotes => remotes::render(frame, app, rows[1]),
        SettingsTab::Install => install::render(frame, app, rows[1]),
    }
}

fn tab_strip(app: &App) -> Line<'_> {
    let mut spans = vec![Span::raw(" ")];
    for (label, tab) in [
        ("Profiles", SettingsTab::Profiles),
        ("Remotes", SettingsTab::Remotes),
        ("Install", SettingsTab::Install),
    ] {
        if app.settings.tab == tab {
            spans.push(Span::styled(format!("[{}] ", label), theme::accent()));
        } else {
            spans.push(Span::styled(format!(" {}  ", label), theme::dim()));
        }
    }
    spans.push(Span::styled("(Tab to switch)", theme::dim()));
    Line::from(spans)
}
