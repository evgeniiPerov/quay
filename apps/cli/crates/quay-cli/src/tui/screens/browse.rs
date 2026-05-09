use crate::tui::app::{App, ScreenAction};
use crate::tui::theme;
use crossterm::event::KeyCode;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum BrowseFocus {
    #[default]
    Remotes,
    Skills,
}

#[derive(Debug, Default)]
pub struct BrowseState {
    pub items: Vec<BrowseRow>,
    pub list_state: ListState,
    pub focus: BrowseFocus,
    pub skill_selected: usize,
}

#[derive(Debug, Clone)]
pub enum BrowseRow {
    Remote { name: String },
}

pub fn handle_key(app: &mut App, code: KeyCode) -> ScreenAction {
    match code {
        KeyCode::Tab | KeyCode::Right | KeyCode::Char('l') => {
            app.browse.focus = match app.browse.focus {
                BrowseFocus::Remotes => BrowseFocus::Skills,
                BrowseFocus::Skills => BrowseFocus::Remotes,
            };
        }
        KeyCode::BackTab | KeyCode::Left | KeyCode::Char('h') => {
            app.browse.focus = match app.browse.focus {
                BrowseFocus::Remotes => BrowseFocus::Skills,
                BrowseFocus::Skills => BrowseFocus::Remotes,
            };
        }
        KeyCode::Down | KeyCode::Char('j') => match app.browse.focus {
            BrowseFocus::Remotes => {
                let max = app.browse.items.len().saturating_sub(1);
                let i = app.browse.list_state.selected().unwrap_or(0);
                app.browse
                    .list_state
                    .select(Some(i.saturating_add(1).min(max)));
                // Reset skill cursor when remote changes.
                app.browse.skill_selected = 0;
            }
            BrowseFocus::Skills => {
                let max = skills_for_selected_remote_count(app).saturating_sub(1);
                app.browse.skill_selected = (app.browse.skill_selected + 1).min(max);
            }
        },
        KeyCode::Up | KeyCode::Char('k') => match app.browse.focus {
            BrowseFocus::Remotes => {
                let i = app.browse.list_state.selected().unwrap_or(0);
                app.browse.list_state.select(Some(i.saturating_sub(1)));
                app.browse.skill_selected = 0;
            }
            BrowseFocus::Skills => {
                app.browse.skill_selected = app.browse.skill_selected.saturating_sub(1);
            }
        },
        KeyCode::Char('a') | KeyCode::Enter => {
            // Install selected skill from selected remote.
            if let Some((skill_name, remote_name)) = selected_skill_and_remote(app) {
                app.defer_blocking_action(crate::tui::app::BlockingAction::Add {
                    skill: skill_name,
                    remote: Some(remote_name),
                });
                app.set_status("installing…");
            }
        }
        _ => {}
    }
    ScreenAction::Stay
}

fn skills_for_selected_remote_count(app: &App) -> usize {
    let Some(remote_name) = current_remote_name(app) else {
        return 0;
    };
    app.search
        .all_skills
        .iter()
        .filter(|sk| sk.remote == remote_name)
        .count()
}

fn current_remote_name(app: &App) -> Option<String> {
    let i = app.browse.list_state.selected()?;
    let row = app.browse.items.get(i)?;
    match row {
        BrowseRow::Remote { name } => Some(name.clone()),
    }
}

fn selected_skill_and_remote(app: &App) -> Option<(String, String)> {
    let remote_name = current_remote_name(app)?;
    let skills: Vec<&crate::tui::screens::search::SkillRow> = app
        .search
        .all_skills
        .iter()
        .filter(|sk| sk.remote == remote_name)
        .collect();
    let sk = skills.get(app.browse.skill_selected)?;
    Some((sk.name.clone(), remote_name))
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

/// Populate `app.browse` directly (not a render-local clone) so key handlers
/// can read the selection. Call on screen entry from the event loop.
pub fn ensure_loaded_into_app(app: &mut App) {
    if !app.browse.items.is_empty() {
        return;
    }
    for rname in app.cfg.remotes.keys() {
        app.browse.items.push(BrowseRow::Remote {
            name: rname.clone(),
        });
    }
    if !app.browse.items.is_empty() {
        app.browse.list_state.select(Some(0));
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
            focus: app.browse.focus,
            skill_selected: app.browse.skill_selected,
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

    let left_title = if app.browse.focus == BrowseFocus::Remotes {
        " Browse ◀ "
    } else {
        " Browse "
    };
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(left_title))
        .highlight_style(theme::selected());
    frame.render_stateful_widget(list, cols[0], &mut local.list_state);

    // Preview pane: skills on the selected remote (filtered from
    // `app.search.all_skills`, which is fetched on screen entry).
    let selected_remote_name: Option<&str> = local
        .list_state
        .selected()
        .and_then(|i| local.items.get(i))
        .map(|row| match row {
            BrowseRow::Remote { name } => name.as_str(),
        });

    let preview_title = match selected_remote_name {
        Some(name) => format!(" Skills on '{}' ", name),
        None => " Preview ".to_string(),
    };

    if !app.search.fetched {
        // Fetch hasn't run yet (e.g. ensure_loaded queued for screen entry).
        frame.render_widget(
            Paragraph::new("(fetching registry.json from configured remotes…)")
                .block(Block::default().borders(Borders::ALL).title(preview_title)),
            cols[1],
        );
        return;
    }

    let skills_for_remote: Vec<&crate::tui::screens::search::SkillRow> = app
        .search
        .all_skills
        .iter()
        .filter(|sk| selected_remote_name.is_some_and(|n| sk.remote == n))
        .collect();

    if skills_for_remote.is_empty() {
        let msg = match selected_remote_name {
            Some(_) => "(no skills on this remote — registry.json empty or unreachable)",
            None => "(no remote selected)",
        };
        frame.render_widget(
            Paragraph::new(msg).block(Block::default().borders(Borders::ALL).title(preview_title)),
            cols[1],
        );
        return;
    }

    let skill_items: Vec<ListItem> = skills_for_remote
        .iter()
        .enumerate()
        .map(|(i, sk)| {
            let prefix =
                if app.browse.focus == BrowseFocus::Skills && i == app.browse.skill_selected {
                    "▌ "
                } else {
                    "  "
                };
            let line = format!("{prefix}{} v{}  — {}", sk.name, sk.version, sk.description);
            if app.browse.focus == BrowseFocus::Skills && i == app.browse.skill_selected {
                use ratatui::style::{Color, Modifier, Style};
                ListItem::new(Line::from(Span::styled(
                    line,
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )))
            } else {
                ListItem::new(Line::from(line))
            }
        })
        .collect();

    let right_title = if app.browse.focus == BrowseFocus::Skills {
        format!("{}◀ ", preview_title.trim_end())
    } else {
        preview_title.clone()
    };

    frame.render_widget(
        List::new(skill_items).block(Block::default().borders(Borders::ALL).title(right_title)),
        cols[1],
    );

    // Bottom hint line (overlaid on the bottom row of cols[1] via a 1-line
    // sub-rect so it fits within the existing layout).
    let hint_area = Rect {
        x: area.x,
        y: area.y + area.height.saturating_sub(1),
        width: area.width,
        height: 1,
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "Tab/h/l switch pane  [j]/[k] move  [a]/Enter install  [q] quit",
            theme::dim(),
        ))),
        hint_area,
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
                push_mode: quay_core::PushMode::default(),
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
