//! TUI Screen [3] — Browse Remote.
//!
//! Fetches one configured remote's `registry.json` and renders rows.
//! Key bindings:
//!   - `[Tab]` — cycle configured remotes
//!   - `[j]/[k]` — navigate rows
//!   - `[Enter]` — preview (SKILL.md content, fetched on demand)
//!   - `[a]` — pull skill (blocks if local copy already exists)
//!   - `[A]` — force pull (overwrite confirm)
//!   - `[r]` — refresh (re-fetch registry.json)

use crate::tui::app::{App, BlockingAction, ScreenAction};
use crate::tui::screens::widgets::spinner::Spinner;
use crate::tui::theme;
use crossterm::event::KeyCode;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use ratatui::Frame;

/// A row loaded from a remote's `registry.json`.
#[derive(Debug, Clone)]
pub struct RemoteSkillRow {
    pub name: String,
    pub version: String,
    pub description: String,
    pub tags: Vec<String>,
}

/// State for the Remote screen.
#[derive(Debug, Default)]
pub struct RemoteState {
    /// Index into the sorted list of configured remote names.
    pub remote_idx: usize,
    /// The rows loaded for the current remote.
    pub rows: Vec<RemoteSkillRow>,
    /// Whether the rows have been fetched for the current remote.
    pub fetched: bool,
    /// Whether a `FetchRegistry` blocking action is in flight.
    pub fetching: bool,
    /// Spinner shown during fetch.
    pub spinner: Spinner,
    /// Selected row index.
    pub selected: usize,
    /// Whether the detail / preview pane is open.
    pub detail_open: bool,
    /// Active confirm modal, if any.
    pub modal: RemoteModal,
    /// A status message local to this screen (overrides global for one render).
    pub local_status: Option<String>,
}

/// Modal overlays for the Remote screen.
#[derive(Debug, Default, Clone)]
pub enum RemoteModal {
    #[default]
    None,
    /// Confirm overwrite for `[A]`.
    ConfirmForcePull { skill_name: String },
}

/// Trigger an async fetch for the currently-selected remote if not yet loaded.
///
/// Sets `fetching = true` and defers a [`BlockingAction::FetchRegistry`].  The
/// spinner will render immediately on the next draw before the blocking clone
/// starts.
pub fn ensure_loaded(app: &mut App) {
    if app.remote.fetched || app.remote.fetching {
        return;
    }
    start_fetch(app);
}

/// Begin an async fetch for the current remote: set `fetching`, clear stale
/// rows, and defer the blocking action.
fn start_fetch(app: &mut App) {
    let remote_names = sorted_remote_names(app);
    let Some(remote_name) = remote_names.get(app.remote.remote_idx).cloned() else {
        // No remote configured — mark fetched with empty rows immediately.
        app.remote.rows = Vec::new();
        app.remote.fetched = true;
        app.remote.fetching = false;
        return;
    };
    app.remote.rows = Vec::new();
    app.remote.fetching = true;
    app.remote.fetched = false;
    app.remote.local_status = None;
    app.set_status("refreshing registry\u{2026}");
    app.defer_blocking_action(BlockingAction::FetchRegistry { remote_name });
}

/// Execute the blocking fetch.  Called from the event loop worker after the
/// spinner has been painted.  Updates `app.remote` in place.
pub fn run_fetch(app: &mut App, remote_name: &str) {
    let Some(remote_cfg) = app.cfg.remotes.get(remote_name) else {
        app.remote.rows = Vec::new();
        app.remote.fetched = true;
        app.remote.fetching = false;
        return;
    };
    let url = remote_cfg.url.clone();

    let mut fetcher = quay_core::CloneFetcher::new();
    match fetcher.fetch_registry(&url).map_err(|e| e.to_string()) {
        Ok(registry) => {
            app.remote.rows = registry
                .skills
                .into_values()
                .map(|e| RemoteSkillRow {
                    name: e.path.trim_start_matches("skills/").to_string(),
                    version: e.version,
                    description: e.description,
                    tags: e.tags,
                })
                .collect();
            app.remote.rows.sort_by(|a, b| a.name.cmp(&b.name));
            app.remote.local_status = None;
            app.set_status(format!(
                "loaded {} skills from {remote_name}",
                app.remote.rows.len()
            ));
        }
        Err(e) => {
            app.remote.local_status = Some(format!("fetch failed: {e}"));
            app.remote.rows = Vec::new();
        }
    }
    app.remote.fetched = true;
    app.remote.fetching = false;
    app.remote.selected = 0;
}

fn sorted_remote_names(app: &App) -> Vec<String> {
    let mut names: Vec<String> = app.cfg.remotes.keys().cloned().collect();
    names.sort();
    names
}

pub fn handle_key(app: &mut App, code: KeyCode) -> ScreenAction {
    // Modal intercept.
    let modal = std::mem::replace(&mut app.remote.modal, RemoteModal::None);
    match modal {
        RemoteModal::ConfirmForcePull { ref skill_name } => {
            match code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    let name = skill_name.clone();
                    pull_skill(app, &name, true);
                }
                _ => {
                    // Restore modal (cancel).
                    app.remote.modal = modal;
                }
            }
            return ScreenAction::Stay;
        }
        RemoteModal::None => {}
    }

    match code {
        KeyCode::Tab => {
            let names = sorted_remote_names(app);
            if names.is_empty() {
                return ScreenAction::Stay;
            }
            app.remote.remote_idx = (app.remote.remote_idx + 1) % names.len();
            app.remote.fetched = false;
            app.remote.selected = 0;
            app.remote.detail_open = false;
            start_fetch(app);
        }
        KeyCode::Char('j') | KeyCode::Down => {
            if !app.remote.rows.is_empty() {
                app.remote.selected =
                    (app.remote.selected + 1).min(app.remote.rows.len().saturating_sub(1));
                app.remote.detail_open = false;
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if !app.remote.rows.is_empty() {
                app.remote.selected = app.remote.selected.saturating_sub(1);
                app.remote.detail_open = false;
            }
        }
        KeyCode::Enter => {
            app.remote.detail_open = !app.remote.detail_open;
        }
        KeyCode::Char('a') => {
            if app.remote.fetching {
                app.set_status("fetch in progress\u{2026} please wait");
            } else if let Some(row) = app.remote.rows.get(app.remote.selected) {
                let name = row.name.clone();
                pull_skill(app, &name, false);
            }
        }
        KeyCode::Char('A') => {
            if app.remote.fetching {
                app.set_status("fetch in progress\u{2026} please wait");
            } else if let Some(row) = app.remote.rows.get(app.remote.selected) {
                let name = row.name.clone();
                app.remote.modal = RemoteModal::ConfirmForcePull { skill_name: name };
            }
        }
        KeyCode::Char('r') => {
            app.remote.fetched = false;
            app.remote.selected = 0;
            start_fetch(app);
        }
        _ => {}
    }
    ScreenAction::Stay
}

fn pull_skill(app: &mut App, skill_name: &str, force: bool) {
    let names = sorted_remote_names(app);
    let remote = names.get(app.remote.remote_idx).cloned();

    if !force {
        // Check for local collision.
        let local_path = app
            .project_root
            .join(".agents/skills")
            .join(skill_name)
            .join("SKILL.md");
        if local_path.exists() {
            app.set_status(format!(
                "'{skill_name}' already exists locally — press [A] to overwrite"
            ));
            return;
        }
    }

    app.defer_blocking_action(BlockingAction::Add {
        skill: skill_name.to_string(),
        remote,
        force,
    });
    app.set_status(format!("pulling {skill_name}…"));
}

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let remote_names = sorted_remote_names(app);
    let current_remote = remote_names.get(app.remote.remote_idx).cloned();

    let rows = &app.remote.rows;
    let selected = app.remote.selected;
    let detail_open = app.remote.detail_open;
    let fetching = app.remote.fetching;

    let (list_area, detail_area) = if detail_open && !fetching {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(area);
        (cols[0], Some(cols[1]))
    } else {
        (area, None)
    };

    let remote_label = current_remote
        .as_deref()
        .unwrap_or("(no remotes configured)");

    if fetching {
        // Show spinner in place of the row list.
        let spinner_frame = app.remote.spinner.frame();
        let title = format!(" Remote: {} ", remote_label);
        let fetching_line = format!(" {} refreshing registry\u{2026}", spinner_frame);
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(fetching_line, theme::accent())))
                .block(Block::default().borders(Borders::ALL).title(title)),
            list_area,
        );
    } else {
        let title = format!(" Remote: {} ({}) ", remote_label, rows.len());

        let selected_style = Style::default()
            .fg(Color::Black)
            .bg(Color::Cyan)
            .add_modifier(Modifier::BOLD);

        let items: Vec<ListItem> = if rows.is_empty() {
            let msg = if app.remote.fetched {
                if let Some(ref local_status) = app.remote.local_status {
                    local_status.as_str()
                } else {
                    "(no skills on this remote)"
                }
            } else {
                "(loading\u{2026})"
            };
            vec![ListItem::new(Line::from(Span::styled(msg, theme::dim())))]
        } else {
            rows.iter()
                .enumerate()
                .map(|(i, row)| {
                    let line = format!(
                        " {:<28}  v{:<12}  {}",
                        truncate(&row.name, 28),
                        truncate(&row.version, 12),
                        truncate(&row.description, 50)
                    );
                    if i == selected {
                        ListItem::new(Line::from(Span::styled(line, selected_style)))
                    } else {
                        ListItem::new(Line::from(line))
                    }
                })
                .collect()
        };

        frame.render_widget(
            List::new(items).block(Block::default().borders(Borders::ALL).title(title)),
            list_area,
        );

        // Detail pane.
        if let Some(det_area) = detail_area {
            if let Some(row) = rows.get(selected) {
                render_detail(frame, det_area, row);
            }
        }
    }

    // Confirm modal overlay.
    if let RemoteModal::ConfirmForcePull { ref skill_name } = app.remote.modal {
        render_confirm_modal(frame, area, skill_name);
    }

    // Hint bar.
    let hint_area = Rect {
        x: area.x,
        y: area.y + area.height.saturating_sub(1),
        width: area.width,
        height: 1,
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "[Tab] cycle remote  [j]/[k] move  [Enter] preview  [a] pull  [A] force pull  [r] refresh",
            theme::dim(),
        ))),
        hint_area,
    );
}

fn render_detail(frame: &mut Frame, area: Rect, row: &RemoteSkillRow) {
    let lines = vec![
        Line::from(Span::styled(
            format!(" {} v{}", row.name, row.version),
            theme::accent(),
        )),
        Line::from(""),
        Line::from(format!(" {}", row.description)),
        Line::from(""),
        Line::from(format!(
            " Tags: {}",
            if row.tags.is_empty() {
                "(none)".to_string()
            } else {
                row.tags.join(", ")
            }
        )),
    ];
    frame.render_widget(
        Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(" Preview ")),
        area,
    );
}

fn render_confirm_modal(frame: &mut Frame, area: Rect, skill_name: &str) {
    use ratatui::widgets::Clear;
    let modal_width = 60_u16;
    let modal_height = 5_u16;
    let modal_x = area.x + area.width.saturating_sub(modal_width) / 2;
    let modal_y = area.y + area.height.saturating_sub(modal_height) / 2;
    let modal_area = Rect {
        x: modal_x,
        y: modal_y,
        width: modal_width.min(area.width),
        height: modal_height,
    };
    frame.render_widget(Clear, modal_area);
    let block = Block::default()
        .borders(Borders::ALL)
        .style(Style::default().bg(Color::DarkGray).fg(Color::White))
        .title(" Confirm Force Pull ");
    let inner = block.inner(modal_area);
    frame.render_widget(block, modal_area);
    let lines = vec![
        Line::from(format!(" Overwrite local '{}'? y/N", skill_name)),
        Line::from(""),
        Line::from(Span::styled(" [y] yes   [any other] cancel", theme::dim())),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max.saturating_sub(1)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quay_core::Config;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    /// Build a minimal bare git repo containing a `registry.json` with one
    /// skill, then verify that `CloneFetcher::fetch_registry` parses it correctly.
    #[test]
    fn clone_fetcher_parses_local_bare_repo() {
        use std::process::Command;

        let work_dir = tempfile::tempdir().unwrap();
        let bare_dir = tempfile::tempdir().unwrap();

        let out = Command::new("git")
            .args(["init", "--bare"])
            .current_dir(bare_dir.path())
            .output()
            .expect("git init --bare");
        assert!(out.status.success(), "git init --bare failed");

        let out = Command::new("git")
            .args(["init"])
            .current_dir(work_dir.path())
            .output()
            .expect("git init");
        assert!(out.status.success(), "git init failed");

        for (k, v) in [("user.email", "test@quay"), ("user.name", "quay-test")] {
            Command::new("git")
                .args(["config", k, v])
                .current_dir(work_dir.path())
                .status()
                .expect("git config");
        }

        let registry_json = r#"{
            "hub": "test-hub",
            "generated_at": "2026-05-10T00:00:00Z",
            "schema_version": 1,
            "skills": {
                "add-entity": {
                    "version": "0.1.0",
                    "description": "Add entity skill",
                    "tags": ["typescript"],
                    "path": "skills/add-entity",
                    "sha": "deadbeef",
                    "files": ["SKILL.md"]
                }
            }
        }"#;
        std::fs::write(work_dir.path().join("registry.json"), registry_json).unwrap();

        Command::new("git")
            .args(["add", "registry.json"])
            .current_dir(work_dir.path())
            .status()
            .expect("git add");
        Command::new("git")
            .args(["commit", "-m", "init"])
            .current_dir(work_dir.path())
            .status()
            .expect("git commit");

        let bare_url = bare_dir.path().to_str().unwrap().to_string();
        Command::new("git")
            .args(["remote", "add", "origin", &bare_url])
            .current_dir(work_dir.path())
            .status()
            .expect("git remote add");
        let branch_out = Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .current_dir(work_dir.path())
            .output()
            .expect("git rev-parse HEAD");
        let branch = String::from_utf8_lossy(&branch_out.stdout)
            .trim()
            .to_string();
        Command::new("git")
            .args(["push", "origin", &format!("{branch}:{branch}")])
            .current_dir(work_dir.path())
            .status()
            .expect("git push");

        let mut fetcher = quay_core::CloneFetcher::new();
        let registry = fetcher
            .fetch_registry(&bare_url)
            .expect("CloneFetcher::fetch_registry should succeed");

        assert_eq!(registry.skills.len(), 1);
        let entry = registry.skills.get("add-entity").unwrap();
        assert_eq!(entry.version, "0.1.0");
        assert_eq!(entry.path, "skills/add-entity");
    }

    fn fixture_app() -> App {
        App::new(Config::default(), std::path::PathBuf::from("/tmp"), None)
    }

    #[test]
    fn remote_renders_without_crash() {
        let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
        let mut a = fixture_app();
        a.current_screen = crate::tui::app::Screen::Remote;
        terminal.draw(|f| crate::tui::draw(f, &a)).unwrap();
    }

    #[test]
    fn remote_renders_with_configured_remote() {
        let mut cfg = Config::default();
        cfg.remotes.insert(
            "team-hub".into(),
            quay_core::RemoteConfig {
                url: "https://x".into(),
                default: true,
                provider: None,
                push_mode: quay_core::PushMode::default(),
            },
        );
        let mut a = App::new(cfg, std::path::PathBuf::from("/tmp"), None);
        a.current_screen = crate::tui::app::Screen::Remote;
        let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
        terminal.draw(|f| crate::tui::draw(f, &a)).unwrap();
        let dump: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(
            dump.contains("team-hub") || dump.contains("Remote"),
            "dump: {dump}"
        );
    }

    #[test]
    fn pull_blocks_when_local_exists() {
        use assert_fs::prelude::*;
        let project = assert_fs::TempDir::new().unwrap();
        project
            .child(".agents/skills/foo/SKILL.md")
            .write_str("---\nname: foo\ndescription: f\n---\n")
            .unwrap();

        let mut cfg = Config::default();
        cfg.remotes.insert(
            "h".into(),
            quay_core::RemoteConfig {
                url: "https://x".into(),
                default: true,
                provider: None,
                push_mode: quay_core::PushMode::default(),
            },
        );
        let mut a = App::new(cfg, project.path().to_path_buf(), None);
        a.remote.rows = vec![RemoteSkillRow {
            name: "foo".into(),
            version: "1.0.0".into(),
            description: "d".into(),
            tags: vec![],
        }];
        a.remote.fetched = true;
        a.remote.selected = 0;

        pull_skill(&mut a, "foo", false);

        // Should have set a status about already existing, NOT deferred a BlockingAction.
        assert!(
            a.next_blocking.is_none(),
            "should not queue action on collision"
        );
        let status = a.status_message.as_deref().unwrap_or("");
        assert!(
            status.contains("already exists") || status.contains("[A]"),
            "expected collision message, got: {status}"
        );
    }

    #[test]
    fn force_pull_queues_blocking_action() {
        let mut cfg = Config::default();
        cfg.remotes.insert(
            "h".into(),
            quay_core::RemoteConfig {
                url: "https://x".into(),
                default: true,
                provider: None,
                push_mode: quay_core::PushMode::default(),
            },
        );
        let mut a = App::new(cfg, std::path::PathBuf::from("/tmp"), None);
        a.remote.rows = vec![RemoteSkillRow {
            name: "foo".into(),
            version: "1.0.0".into(),
            description: "d".into(),
            tags: vec![],
        }];
        a.remote.fetched = true;
        a.remote.selected = 0;

        pull_skill(&mut a, "foo", true);

        assert!(
            a.next_blocking.is_some(),
            "force pull must queue a BlockingAction"
        );
    }

    /// `ensure_loaded` on a remote with a configured URL defers a
    /// `FetchRegistry` blocking action and sets `fetching = true`.
    #[test]
    fn ensure_loaded_defers_fetch_registry_action() {
        let mut cfg = Config::default();
        cfg.remotes.insert(
            "hub".into(),
            quay_core::RemoteConfig {
                url: "https://example.com/hub.git".into(),
                default: true,
                provider: None,
                push_mode: quay_core::PushMode::default(),
            },
        );
        let mut a = App::new(cfg, std::path::PathBuf::from("/tmp"), None);
        assert!(!a.remote.fetched, "should start unfetched");

        ensure_loaded(&mut a);

        assert!(a.remote.fetching, "fetching flag must be set");
        assert!(
            a.next_blocking.is_some(),
            "FetchRegistry action must be queued"
        );
        if let Some(crate::tui::app::BlockingAction::FetchRegistry { ref remote_name }) =
            a.next_blocking
        {
            assert_eq!(remote_name, "hub");
        } else {
            panic!("expected FetchRegistry action");
        }
    }

    /// `ensure_loaded` is a no-op when already fetching (avoids double-queue).
    #[test]
    fn ensure_loaded_noop_when_already_fetching() {
        let mut cfg = Config::default();
        cfg.remotes.insert(
            "hub".into(),
            quay_core::RemoteConfig {
                url: "https://example.com/hub.git".into(),
                default: true,
                provider: None,
                push_mode: quay_core::PushMode::default(),
            },
        );
        let mut a = App::new(cfg, std::path::PathBuf::from("/tmp"), None);
        a.remote.fetching = true;

        ensure_loaded(&mut a);

        assert!(
            a.next_blocking.is_none(),
            "must not queue a second action while already fetching"
        );
    }

    /// `[a]` while fetching sets a "fetch in progress" status and does NOT
    /// queue a BlockingAction::Add.
    #[test]
    fn pull_blocked_while_fetching() {
        let mut cfg = Config::default();
        cfg.remotes.insert(
            "hub".into(),
            quay_core::RemoteConfig {
                url: "https://x".into(),
                default: true,
                provider: None,
                push_mode: quay_core::PushMode::default(),
            },
        );
        let mut a = App::new(cfg, std::path::PathBuf::from("/tmp"), None);
        a.remote.fetching = true;
        a.remote.rows = vec![RemoteSkillRow {
            name: "foo".into(),
            version: "1.0.0".into(),
            description: "d".into(),
            tags: vec![],
        }];
        a.remote.selected = 0;

        handle_key(&mut a, KeyCode::Char('a'));

        assert!(
            a.next_blocking.is_none(),
            "must not queue Add while fetching"
        );
        let status = a.status_message.as_deref().unwrap_or("");
        assert!(
            status.contains("fetch in progress") || status.contains("please wait"),
            "expected in-progress message, got: {status}"
        );
    }

    /// Render while fetching shows the spinner widget without crashing.
    #[test]
    fn remote_renders_spinner_while_fetching() {
        let mut cfg = Config::default();
        cfg.remotes.insert(
            "hub".into(),
            quay_core::RemoteConfig {
                url: "https://x".into(),
                default: true,
                provider: None,
                push_mode: quay_core::PushMode::default(),
            },
        );
        let mut a = App::new(cfg, std::path::PathBuf::from("/tmp"), None);
        a.current_screen = crate::tui::app::Screen::Remote;
        a.remote.fetching = true;

        let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
        terminal.draw(|f| crate::tui::draw(f, &a)).unwrap();
        let dump: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(
            dump.contains("refreshing") || dump.contains("hub"),
            "spinner render: {dump}"
        );
    }

    /// `[Tab]` cycling resets `fetched` and starts a new async fetch.
    #[test]
    fn tab_cycles_remote_and_starts_fetch() {
        let mut cfg = Config::default();
        for name in ["alpha", "beta"] {
            cfg.remotes.insert(
                name.into(),
                quay_core::RemoteConfig {
                    url: format!("https://{name}.example.com/hub.git"),
                    default: false,
                    provider: None,
                    push_mode: quay_core::PushMode::default(),
                },
            );
        }
        let mut a = App::new(cfg, std::path::PathBuf::from("/tmp"), None);
        a.remote.fetched = true;
        a.remote.remote_idx = 0;

        handle_key(&mut a, KeyCode::Tab);

        assert_eq!(a.remote.remote_idx, 1, "remote_idx must advance");
        assert!(!a.remote.fetched, "fetched must be cleared on Tab");
        assert!(a.remote.fetching, "fetching must be set on Tab");
        assert!(
            a.next_blocking.is_some(),
            "FetchRegistry must be queued on Tab"
        );
    }
}
