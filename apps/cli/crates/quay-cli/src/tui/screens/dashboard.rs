use crate::tui::app::{App, BlockingAction, Screen, ScreenAction};
use crate::tui::theme;
use crossterm::event::KeyCode;
use quay_core::BumpKind;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use ratatui::Frame;

pub fn handle_key(app: &mut App, code: KeyCode) -> ScreenAction {
    match code {
        // Plan 10: [2] Local, [3] Remote replace old b/i/c shortcuts.
        KeyCode::Char('r') => {
            app.reload_local_skills();
            ScreenAction::Stay
        }
        KeyCode::Char('j') | KeyCode::Down => {
            if !app.local_skills.is_empty() {
                app.local_selected =
                    (app.local_selected + 1).min(app.local_skills.len().saturating_sub(1));
            }
            ScreenAction::Stay
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if !app.local_skills.is_empty() {
                app.local_selected = app.local_selected.saturating_sub(1);
            }
            ScreenAction::Stay
        }
        KeyCode::Char('u') => {
            if let Some(skill) = app.local_skills.get(app.local_selected) {
                let name = skill.meta.name.clone();
                app.create_push = crate::tui::screens::create_push::CreatePushState::Pushing {
                    skill: name.clone(),
                    remote: None,
                    bump: crate::tui::screens::create_push::BumpChoice::AsWritten,
                    started_at: std::time::Instant::now(),
                    spinner: crate::tui::screens::widgets::spinner::Spinner::default(),
                };
                app.push_form_ready = true;
                app.defer_blocking_action(BlockingAction::Push {
                    skill: name,
                    remote: None,
                    bump: BumpKind::AsWritten,
                });
                return ScreenAction::SwitchTo(Screen::CreatePush);
            }
            ScreenAction::Stay
        }
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
            Constraint::Percentage(50),
            Constraint::Percentage(50),
            Constraint::Length(2),
        ])
        .split(inner);

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Dashboard", theme::accent()),
            Span::raw("    "),
            Span::styled(
                "[2] Local  [3] Remote  [s] Search  [r] rescan  [u] push  [,] Settings  [q] quit",
                theme::dim(),
            ),
        ])),
        rows[0],
    );

    let top = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(rows[1]);

    // Remotes (top-left)
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
        top[0],
    );

    // Local skills summary (top-right)
    let local_summary_items: Vec<ListItem> = app
        .local_skills
        .iter()
        .map(|s| ListItem::new(Line::from(format!("{} v{}", s.meta.name, s.meta.version))))
        .collect();
    frame.render_widget(
        List::new(local_summary_items).block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" Installed ({}) ", app.local_skills.len())),
        ),
        top[1],
    );

    let bottom = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(rows[2]);

    // Local skills (bottom-left)
    let local_items: Vec<ListItem> = if app.local_skills.is_empty() {
        vec![ListItem::new(Line::from(Span::styled(
            "(no local skills under .agents/skills/)",
            theme::dim(),
        )))]
    } else {
        use ratatui::style::{Color, Modifier, Style};
        let selected_style = Style::default()
            .fg(Color::Black)
            .bg(Color::Cyan)
            .add_modifier(Modifier::BOLD);
        app.local_skills
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let badge = badge_for(&s.status);
                let prefix = if i == app.local_selected {
                    "▌ "
                } else {
                    "  "
                };
                let line = format!("{prefix}{:<24}  {}", s.meta.name, badge);
                if i == app.local_selected {
                    ListItem::new(Line::from(Span::styled(line, selected_style)))
                } else {
                    ListItem::new(Line::from(line))
                }
            })
            .collect()
    };
    frame.render_widget(
        List::new(local_items).block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" Local skills ({}) ", app.local_skills.len())),
        ),
        bottom[0],
    );

    // Outdated (bottom-right) — placeholder per existing UX
    frame.render_widget(
        List::new(vec![ListItem::new(Line::from(Span::styled(
            "(run `quay outdated` from CLI)",
            theme::dim(),
        )))])
        .block(Block::default().borders(Borders::ALL).title(" Outdated ")),
        bottom[1],
    );

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "Tip: [1] Dashboard  [2] Local  [3] Remote  [s] Search  [,] Settings  [j]/[k] move  [r] rescan  [u] push  [q] quit",
            theme::dim(),
        ))),
        rows[3],
    );
}

fn badge_for(status: &quay_core::scanner::ScanStatus) -> String {
    use quay_core::scanner::ScanStatus;
    match status {
        ScanStatus::Local => "◌ local".to_string(),
        ScanStatus::Installed { version, .. } => format!("◉ installed v{version}"),
        ScanStatus::InstalledModified { version, .. } => format!("⚠ modified v{version}"),
        ScanStatus::PushedLocal { pr_url, .. } if pr_url.is_empty() => {
            "↑ pushed-direct".to_string()
        }
        ScanStatus::PushedLocal { .. } => "↑ pushed-local".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quay_core::{Config, RemoteConfig};
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
                push_mode: quay_core::PushMode::default(),
                direct_branch: None,
            },
        );
        cfg.user.email = Some("dev@example.com".into());
        App::new(cfg, std::path::PathBuf::from("/tmp"), None)
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
        assert!(dump.contains("Installed ("), "buffer: {}", dump);
        assert!(dump.contains("Dashboard"));
    }

    #[test]
    fn dashboard_b_key_no_longer_switches_to_browse() {
        // Plan 10: [b] is removed from the Dashboard. Use global [3] for Remote.
        let mut app = fixture_app();
        let action = handle_key(&mut app, KeyCode::Char('b'));
        // Should stay — no longer a shortcut.
        assert!(
            matches!(action, ScreenAction::Stay),
            "expected Stay, got {:?}",
            action
        );
    }

    #[test]
    fn dashboard_c_key_no_longer_switches_to_create_push() {
        // Plan 10: [c] is removed from Dashboard. Use [u] from Local screen instead.
        let mut app = fixture_app();
        let action = handle_key(&mut app, KeyCode::Char('c'));
        assert!(
            matches!(action, ScreenAction::Stay),
            "expected Stay, got {:?}",
            action
        );
    }

    #[test]
    fn dashboard_picks_up_local_skill_via_app_scan() {
        use assert_fs::prelude::*;
        let project = assert_fs::TempDir::new().unwrap();
        project
            .child(".agents/skills/foo/SKILL.md")
            .write_str("---\nname: foo\ndescription: f\n---\n")
            .unwrap();

        let mut cfg = Config::default();
        cfg.user.email = Some("dev@example.com".into());
        let app = App::new(cfg, project.path().to_path_buf(), None);
        assert_eq!(app.local_skills.len(), 1);
        assert_eq!(app.local_skills[0].meta.name, "foo");
    }

    #[test]
    fn dashboard_rescan_after_filesystem_change_picks_up_new_skill() {
        use assert_fs::prelude::*;
        let project = assert_fs::TempDir::new().unwrap();
        project
            .child(".agents/skills/foo/SKILL.md")
            .write_str("---\nname: foo\ndescription: f\n---\n")
            .unwrap();

        let mut cfg = Config::default();
        cfg.user.email = Some("dev@example.com".into());
        let mut app = App::new(cfg, project.path().to_path_buf(), None);
        assert_eq!(app.local_skills.len(), 1);

        project
            .child(".agents/skills/bar/SKILL.md")
            .write_str("---\nname: bar\ndescription: b\n---\n")
            .unwrap();
        app.reload_local_skills();
        assert_eq!(app.local_skills.len(), 2);
    }

    #[test]
    fn dashboard_render_shows_local_skill_panel_title() {
        use assert_fs::prelude::*;
        let project = assert_fs::TempDir::new().unwrap();
        project
            .child(".agents/skills/foo/SKILL.md")
            .write_str("---\nname: foo\ndescription: f\n---\n")
            .unwrap();

        let mut cfg = Config::default();
        cfg.user.email = Some("dev@example.com".into());
        let app = App::new(cfg, project.path().to_path_buf(), None);

        let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
        terminal.draw(|f| crate::tui::draw(f, &app)).unwrap();
        let dump: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(
            dump.contains("Local skills"),
            "missing 'Local skills' panel title"
        );
        assert!(dump.contains("foo"), "missing 'foo' skill row");
    }
}
