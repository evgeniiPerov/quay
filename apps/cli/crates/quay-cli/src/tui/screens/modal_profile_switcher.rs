//! Profile switcher modal — opened by `p` from any screen.

use crate::config_io::{read_user_file, write_user_file};
use crate::tui::app::{App, ModalState};
use crate::tui::theme;
use crossterm::event::KeyCode;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState};
use ratatui::Frame;

/// State held by the profile switcher modal while it is open.
#[derive(Debug, Default)]
pub struct SwitcherState {
    /// Which row is highlighted in the profile list.
    pub list_state: ListState,
}

/// Open the profile switcher modal, pre-selecting the currently active profile.
pub fn open(app: &mut App) {
    let mut s = SwitcherState::default();
    let path = app.user_config_path.as_deref();
    let file = read_user_file(path).ok();
    let active = file.as_ref().and_then(|f| f.active_profile.clone());
    if let (Some(file), Some(active)) = (file, active) {
        let i = file.profiles.keys().position(|k| k == &active);
        s.list_state.select(i.or(Some(0)));
    } else {
        s.list_state.select(Some(0));
    }
    app.modal = Some(ModalState::ProfileSwitcher(s));
}

/// Route a key event to the modal. Must be called only when `app.modal` is `Some`.
pub fn handle_key(app: &mut App, code: KeyCode) {
    let path = app.user_config_path.as_deref();
    let file = read_user_file(path).unwrap_or_default();
    let names: Vec<String> = file.profiles.keys().cloned().collect();

    let state = match &mut app.modal {
        Some(ModalState::ProfileSwitcher(s)) => s,
        _ => return,
    };

    match code {
        KeyCode::Esc => {
            app.modal = None;
        }
        KeyCode::Down | KeyCode::Char('j') => {
            let max = names.len().saturating_sub(1);
            let i = state.list_state.selected().unwrap_or(0);
            state.list_state.select(Some(i.saturating_add(1).min(max)));
        }
        KeyCode::Up | KeyCode::Char('k') => {
            let i = state.list_state.selected().unwrap_or(0);
            state.list_state.select(Some(i.saturating_sub(1)));
        }
        KeyCode::Enter => {
            let chosen = state
                .list_state
                .selected()
                .and_then(|i| names.get(i).cloned());
            if let Some(name) = chosen {
                let mut file = file;
                file.active_profile = Some(name.clone());
                if let Some(p) = path {
                    if let Err(e) = write_user_file(p, &file) {
                        app.set_status(format!("error: {}", e));
                        app.modal = None;
                        return;
                    }
                }
                app.set_status(format!("active profile: {}", name));
            }
            app.modal = None;
        }
        _ => {}
    }
}

/// Render the modal overlay on top of whatever is behind it.
pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let state = match &app.modal {
        Some(ModalState::ProfileSwitcher(s)) => s,
        _ => return,
    };
    let path = app.user_config_path.as_deref();
    let file = read_user_file(path).unwrap_or_default();
    let active = file.active_profile.clone();

    let items: Vec<ListItem> = file
        .profiles
        .keys()
        .map(|name| {
            let marker = if active.as_deref() == Some(name.as_str()) {
                "▶ "
            } else {
                "  "
            };
            ListItem::new(Line::from(format!("{}{}", marker, name)))
        })
        .collect();

    let modal_area = centered_rect(area, 40, 40);
    frame.render_widget(Clear, modal_area);
    let outer = Block::default()
        .borders(Borders::ALL)
        .title(" Switch profile ");
    let inner = outer.inner(modal_area);
    frame.render_widget(outer, modal_area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(inner);

    let mut list_state = state.list_state.clone();
    frame.render_stateful_widget(
        List::new(items).highlight_style(theme::selected()),
        rows[0],
        &mut list_state,
    );
    frame.render_widget(
        ratatui::widgets::Paragraph::new("↑/↓ select   Enter — use   Esc — cancel"),
        rows[1],
    );
}

/// Return a centered rectangle that is `percent_x`% wide and `percent_y`% tall
/// within `area`.
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
    use quay_core::{Config, ProfileFile};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn app_with_two_profiles() -> (App, assert_fs::TempDir) {
        let dir = assert_fs::TempDir::new().unwrap();
        let user_path = dir.child("user.toml");
        let mut file = quay_core::UserConfigFile {
            active_profile: Some("work".into()),
            ..Default::default()
        };
        file.profiles.insert("work".into(), ProfileFile::default());
        file.profiles
            .insert("personal".into(), ProfileFile::default());
        write_user_file(user_path.path(), &file).unwrap();

        let a = App::new(
            Config::default(),
            dir.path().to_path_buf(),
            Some(user_path.path().to_path_buf()),
        );
        (a, dir)
    }

    #[test]
    fn open_then_render_shows_both_profiles() {
        let (mut a, _dir) = app_with_two_profiles();
        open(&mut a);
        let mut term = Terminal::new(TestBackend::new(80, 20)).unwrap();
        term.draw(|f| {
            crate::tui::draw(f, &a);
        })
        .unwrap();
        let dump: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(dump.contains("work"));
        assert!(dump.contains("personal"));
    }

    #[test]
    fn enter_switches_active_profile_on_disk() {
        let (mut a, dir) = app_with_two_profiles();
        open(&mut a);
        // Active profile is "work" (index 1 in BTreeMap order). Navigate Up to
        // "personal" (index 0), then press Enter to switch.
        handle_key(&mut a, KeyCode::Up);
        handle_key(&mut a, KeyCode::Enter);
        let written = std::fs::read_to_string(dir.child("user.toml").path()).unwrap();
        assert!(
            written.contains("active_profile = \"personal\""),
            "wrote: {}",
            written
        );
        assert!(a.modal.is_none());
    }
}
