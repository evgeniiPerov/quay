//! TUI entry point and event loop.

pub mod app;
pub mod editor;
pub mod form_theme;
pub mod screens;
pub mod theme;
pub mod widgets;

use crate::commands;
use crate::config_io;
use crate::tui::app::{App, BlockingAction, Screen, ScreenAction};
use crate::tui::screens::onboarding::OnboardingState;
use crossterm::event::KeyCode as KCode;
use crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    Event, KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::io;
use std::path::Path;
use std::time::{Duration, Instant};

pub type TuiResult<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// Determine which screen to start on given the user config path.
///
/// - Path `None` → dashboard (no config file location configured).
/// - File missing or unreadable with a `Some` path → onboarding.
/// - File present but `should_show_onboarding` returns true → onboarding.
/// - Otherwise → dashboard.
///
/// This is the public entry-point used by tests (Task 12).
pub fn decide_initial_screen(user_config_path: Option<&Path>) -> Screen {
    // No config path provided: cannot run onboarding (nowhere to write result).
    if user_config_path.is_none() {
        return Screen::Dashboard;
    }
    match config_io::read_user_file(user_config_path) {
        Ok(file) if app::should_show_onboarding(&file) => Screen::Onboarding,
        Ok(_) => Screen::Dashboard,
        Err(_) => Screen::Onboarding,
    }
}

pub fn run(mut app: App) -> TuiResult<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    execute!(stdout, EnableBracketedPaste)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = event_loop(&mut terminal, &mut app);

    let _ = execute!(terminal.backend_mut(), DisableBracketedPaste);
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    result
}

fn event_loop<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
) -> TuiResult<()> {
    let tick_rate = Duration::from_millis(250);
    let mut last_tick = Instant::now();
    // Pending `g`-prefix chord key.
    let mut pending_g = false;

    while !app.should_quit {
        terminal.draw(|frame| draw(frame, app))?;

        // After rendering, run any deferred blocking action.  This guarantees
        // the spinner (or other in-progress state) is painted at least once
        // before the blocking call freezes the event loop.
        if let Some(action) = app.next_blocking.take() {
            run_blocking_action(action, app);
        }

        let timeout = tick_rate
            .checked_sub(last_tick.elapsed())
            .unwrap_or(Duration::ZERO);
        if event::poll(timeout)? {
            match event::read()? {
                Event::Key(key) => {
                    let action = handle_key(app, key.code, &mut pending_g);
                    if let ScreenAction::SwitchTo(s) = &action {
                        app.switch_to(*s);
                        match *s {
                            Screen::Search => {
                                screens::search::ensure_loaded(app);
                            }
                            Screen::Browse => {
                                // Legacy Browse screen — also loads search data.
                                screens::search::ensure_loaded(app);
                                screens::browse::ensure_loaded_into_app(app);
                            }
                            Screen::Remote => {
                                screens::remote::ensure_loaded(app);
                            }
                            Screen::CreatePush => {
                                if app.push_form_ready {
                                    // State was pre-populated by Local [u]/[U] — leave it as-is.
                                    app.push_form_ready = false;
                                } else {
                                    // Normal entry (e.g. g+c chord) — reset to a fresh create form.
                                    app.create_push = screens::create_push::CreatePushState::Form(
                                        screens::create_push::build_create_form_from_app(app),
                                    );
                                }
                            }
                            _ => {}
                        }
                    } else if let ScreenAction::Quit = &action {
                        app.should_quit = true;
                    }
                }
                Event::Paste(s) => {
                    app.handle_paste(&s);
                }
                _ => {}
            }
        }
        if last_tick.elapsed() >= tick_rate {
            // Tick: advance any animated state.
            app.create_push.tick();
            app.remote.bulk_add.tick();
            if app.settings.remotes.testing_idx.is_some() {
                app.settings.remotes.spinner.advance();
            }
            if app.remote.fetching {
                app.remote.spinner.advance();
            }
            last_tick = Instant::now();
        }

        // After handling events / ticks, check if the onboarding screen has
        // finished (Save or Skip wrote the config).  If so, reload the user
        // config and switch to the dashboard.
        if app.current_screen == Screen::Onboarding
            && matches!(app.onboarding, OnboardingState::Saving)
        {
            // Reload config from disk so the dashboard sees the new profile.
            if let Some(path) = app.user_config_path.clone() {
                if let Ok(cfg) = quay_core::Config::load_resolved(Some(&path), None, None) {
                    app.cfg = cfg;
                }
            }
            app.onboarding = OnboardingState::default();
            app.switch_to(Screen::Dashboard);
        }
    }
    Ok(())
}

/// Execute a blocking action synchronously and update the app state.
fn run_blocking_action(action: BlockingAction, app: &mut App) {
    match action {
        BlockingAction::TestConnection {
            url,
            kind,
            remote_idx,
        } => {
            let provider = quay_core::provider_for_remote(&url, kind);
            let status = provider
                .test_connection(&url)
                .unwrap_or_else(|e| quay_core::ConnectionStatus::Unreachable(e.to_string()));
            app.settings.remotes.testing_idx = None;
            app.settings.remotes.last_results.insert(remote_idx, status);
        }
        BlockingAction::Add {
            skill,
            remote,
            force,
        } => {
            use crate::tui::screens::remote::BulkAddState;
            use quay_core::{CloneFetcher, Config, SkillManager};
            let project_config = app.project_root.join(".quay/config.toml");
            let project_path_arg = if project_config.exists() {
                Some(project_config.as_path())
            } else {
                None
            };
            let cfg_res =
                Config::load_resolved(app.user_config_path.as_deref(), project_path_arg, None);
            let cfg = match cfg_res {
                Ok(c) => c,
                Err(e) => {
                    app.set_status(format!("install failed: {e}"));
                    return;
                }
            };
            let f = CloneFetcher::new();
            let mgr = SkillManager::new(&cfg, &f, &f, app.project_root.clone());
            let result = if force {
                mgr.add_with_force(&skill, remote.as_deref(), true)
            } else {
                mgr.add(&skill, remote.as_deref())
            };

            // Check if we're in bulk-add mode; if so, advance the state machine.
            let in_bulk = matches!(app.remote.bulk_add, BulkAddState::BulkAdding { .. });
            if in_bulk {
                let (ok, err_msg) = match &result {
                    Ok(()) => (true, None),
                    Err(e) => (false, Some(e.to_string())),
                };
                if result.is_ok() {
                    app.reload_local_skills();
                }
                screens::remote::advance_bulk_add(app, skill, ok, err_msg);
            } else {
                match result {
                    Ok(()) => {
                        app.reload_local_skills();
                        app.set_status(format!("installed {}", skill));
                    }
                    Err(quay_core::QuayError::AlreadyExists(_)) => {
                        app.set_status(format!(
                            "'{skill}' already exists locally — press [A] to overwrite"
                        ));
                    }
                    Err(e) => {
                        app.set_status(format!("install failed: {e}"));
                    }
                }
            }
        }
        BlockingAction::Push {
            skill,
            remote,
            bump,
        } => {
            let result = commands::push::push_skill(
                &skill,
                remote.as_deref(),
                bump,
                None,  // push_mode: use remote default
                None,  // direct_branch: use remote config value
                None,  // profile
                &app.project_root,
                app.user_config_path.as_deref(),
            );
            match result {
                Ok(outcome) => {
                    app.create_push = screens::create_push::CreatePushState::Done(outcome);
                    app.reload_local_skills();
                }
                Err(e) => {
                    let placeholder = screens::create_push::CreatePushState::Form(
                        screens::create_push::build_create_form(&[]),
                    );
                    let pushing_state = std::mem::replace(&mut app.create_push, placeholder);
                    app.create_push = screens::create_push::CreatePushState::Failed {
                        state: Box::new(pushing_state),
                        message: e.to_string(),
                    };
                }
            }
        }
        BlockingAction::FetchRegistry { remote_name } => {
            screens::remote::run_fetch(app, &remote_name);
        }
    }
}

/// Translate a pasted string into synthetic [`KeyEvent`]s that form handlers
/// can accept as plain typed characters.
///
/// Filters out `\r` and `\n` — newlines in a paste would otherwise trigger
/// Enter (form submit), which is never what the user wants when pasting a URL.
pub fn paste_to_key_events(s: &str) -> Vec<KeyEvent> {
    s.chars()
        .filter(|c| *c != '\r' && *c != '\n')
        .map(|c| KeyEvent {
            code: KCode::Char(c),
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        })
        .collect()
}

pub fn handle_key(app: &mut App, code: KeyCode, pending_g: &mut bool) -> ScreenAction {
    if app.modal.is_some() {
        screens::modal_profile_switcher::handle_key(app, code);
        return ScreenAction::Stay;
    }

    // When a text input is focused, every keystroke must reach the form verbatim.
    if app.has_focused_text_input() {
        *pending_g = false;
        return match app.current_screen {
            Screen::Onboarding => screens::onboarding::handle_key(app, code),
            Screen::CreatePush => screens::create_push::handle_key(app, code),
            Screen::Settings => screens::settings::handle_key(app, code),
            Screen::Dashboard => screens::dashboard::handle_key(app, code),
            Screen::Local => screens::local::handle_key(app, code),
            Screen::Remote => screens::remote::handle_key(app, code),
            Screen::Browse => screens::browse::handle_key(app, code),
            Screen::Search => screens::search::handle_key(app, code),
            Screen::Installed => screens::installed::handle_key(app, code),
        };
    }

    // Handle the `g`-prefix chord table.
    if *pending_g {
        *pending_g = false;
        let action = match code {
            KeyCode::Char('d') => ScreenAction::SwitchTo(Screen::Dashboard),
            KeyCode::Char('l') => ScreenAction::SwitchTo(Screen::Local),
            KeyCode::Char('r') => ScreenAction::SwitchTo(Screen::Remote),
            KeyCode::Char('s') => ScreenAction::SwitchTo(Screen::Settings),
            // Legacy chords kept for backward compat with existing tests.
            KeyCode::Char('b') => ScreenAction::SwitchTo(Screen::Browse),
            KeyCode::Char('i') => ScreenAction::SwitchTo(Screen::Installed),
            KeyCode::Char('c') => ScreenAction::SwitchTo(Screen::CreatePush),
            _ => ScreenAction::Stay,
        };
        return action;
    }

    // Global keys.
    match code {
        KeyCode::Char('q') => return ScreenAction::Quit,
        // Plan 10 navigation: [1] Dashboard, [2] Local, [3] Remote, [s] Search, [,] Settings.
        KeyCode::Char('1') => return ScreenAction::SwitchTo(Screen::Dashboard),
        KeyCode::Char('2') => return ScreenAction::SwitchTo(Screen::Local),
        KeyCode::Char('3') => return ScreenAction::SwitchTo(Screen::Remote),
        KeyCode::Char('s') if app.current_screen != Screen::Search => {
            return ScreenAction::SwitchTo(Screen::Search);
        }
        KeyCode::Char(',') => return ScreenAction::SwitchTo(Screen::Settings),
        KeyCode::Char('g') => {
            *pending_g = true;
            return ScreenAction::Stay;
        }
        KeyCode::Char('p') => {
            screens::modal_profile_switcher::open(app);
            return ScreenAction::Stay;
        }
        _ => {}
    }

    match app.current_screen {
        Screen::Dashboard => screens::dashboard::handle_key(app, code),
        Screen::Local => screens::local::handle_key(app, code),
        Screen::Remote => screens::remote::handle_key(app, code),
        Screen::Browse => screens::browse::handle_key(app, code),
        Screen::Search => screens::search::handle_key(app, code),
        Screen::Installed => screens::installed::handle_key(app, code),
        Screen::Settings => screens::settings::handle_key(app, code),
        Screen::CreatePush => screens::create_push::handle_key(app, code),
        Screen::Onboarding => screens::onboarding::handle_key(app, code),
    }
}

pub fn draw(frame: &mut ratatui::Frame, app: &App) {
    use ratatui::layout::{Constraint, Direction, Layout};

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(frame.area());

    match app.current_screen {
        Screen::Dashboard => screens::dashboard::render(frame, app, chunks[0]),
        Screen::Local => screens::local::render(frame, app, chunks[0]),
        Screen::Remote => screens::remote::render(frame, app, chunks[0]),
        Screen::Browse => screens::browse::render(frame, app, chunks[0]),
        Screen::Search => screens::search::render(frame, app, chunks[0]),
        Screen::Installed => screens::installed::render(frame, app, chunks[0]),
        Screen::Settings => screens::settings::render(frame, app, chunks[0]),
        Screen::CreatePush => screens::create_push::render(frame, app, chunks[0], &app.create_push),
        Screen::Onboarding => screens::onboarding::render(frame, app, chunks[0], &app.onboarding),
    }
    widgets::render_status_bar(frame, app, chunks[1]);
    if app.modal.is_some() {
        screens::modal_profile_switcher::render(frame, app, frame.area());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::app::App;
    use crossterm::event::KeyCode as KCode;
    use quay_core::Config;

    fn fixture_app() -> App {
        App::new(Config::default(), std::path::PathBuf::from("/tmp"), None)
    }

    // -----------------------------------------------------------------------
    // paste_to_key_events
    // -----------------------------------------------------------------------

    #[test]
    fn paste_to_key_events_drops_newlines() {
        let events = paste_to_key_events("a\nb\rc");
        assert_eq!(events.len(), 3);
        assert!(events.iter().all(|e| matches!(e.code, KCode::Char(_))));
    }

    #[test]
    fn paste_to_key_events_preserves_other_chars() {
        let events = paste_to_key_events("git@github.com:o/r.git");
        let s: String = events
            .iter()
            .filter_map(|e| {
                if let KCode::Char(c) = e.code {
                    Some(c)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(s, "git@github.com:o/r.git");
    }

    #[test]
    fn paste_to_key_events_handles_empty() {
        assert!(paste_to_key_events("").is_empty());
    }

    // -----------------------------------------------------------------------
    // Task 12 — startup gate tests
    // -----------------------------------------------------------------------

    #[test]
    fn onboarding_when_no_config_file() {
        let dir = assert_fs::TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        let initial = decide_initial_screen(Some(&path));
        assert!(
            matches!(initial, Screen::Onboarding),
            "expected Onboarding, got {:?}",
            initial
        );
    }

    #[test]
    fn onboarding_when_meta_false_and_no_profiles() {
        let dir = assert_fs::TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "").unwrap();
        let initial = decide_initial_screen(Some(&path));
        assert!(
            matches!(initial, Screen::Onboarding),
            "expected Onboarding, got {:?}",
            initial
        );
    }

    #[test]
    fn dashboard_when_existing_profiles() {
        let dir = assert_fs::TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "active_profile = \"p\"\n[profiles.p.user]\nemail = \"x@y\"\n",
        )
        .unwrap();
        let initial = decide_initial_screen(Some(&path));
        assert!(
            matches!(initial, Screen::Dashboard),
            "expected Dashboard, got {:?}",
            initial
        );
    }

    /// `onboarded = true` with no profiles still routes to onboarding.
    #[test]
    fn onboarding_when_skipped_marker_but_no_profiles() {
        let dir = assert_fs::TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[meta]\nonboarded = true\n").unwrap();
        let initial = decide_initial_screen(Some(&path));
        assert!(
            matches!(initial, Screen::Onboarding),
            "expected Onboarding, got {:?}",
            initial
        );
    }

    // -----------------------------------------------------------------------
    // Chord / key tests
    // -----------------------------------------------------------------------

    #[test]
    fn g_c_chord_enters_create_push() {
        let mut app = fixture_app();
        let mut pending_g = false;
        let action = handle_key(&mut app, KeyCode::Char('g'), &mut pending_g);
        assert!(matches!(action, ScreenAction::Stay));
        assert!(pending_g, "pending_g should be true after 'g'");
        let action = handle_key(&mut app, KeyCode::Char('c'), &mut pending_g);
        assert!(
            matches!(action, ScreenAction::SwitchTo(Screen::CreatePush)),
            "expected SwitchTo(CreatePush), got {:?}",
            action
        );
        assert!(!pending_g, "pending_g should be cleared after chord");
    }

    #[test]
    fn g_s_chord_enters_settings() {
        let mut app = fixture_app();
        let mut pending_g = false;
        handle_key(&mut app, KeyCode::Char('g'), &mut pending_g);
        let action = handle_key(&mut app, KeyCode::Char('s'), &mut pending_g);
        assert!(matches!(action, ScreenAction::SwitchTo(Screen::Settings)));
    }

    #[test]
    fn g_unknown_chord_stays() {
        let mut app = fixture_app();
        let mut pending_g = false;
        handle_key(&mut app, KeyCode::Char('g'), &mut pending_g);
        let action = handle_key(&mut app, KeyCode::Char('z'), &mut pending_g);
        assert!(matches!(action, ScreenAction::Stay));
        assert!(!pending_g, "pending_g must be cleared on unknown chord");
    }

    // -----------------------------------------------------------------------
    // Plan 10 — new screen jump keys
    // -----------------------------------------------------------------------

    #[test]
    fn key_1_jumps_to_dashboard() {
        let mut app = fixture_app();
        let mut pending_g = false;
        let action = handle_key(&mut app, KeyCode::Char('1'), &mut pending_g);
        assert!(matches!(action, ScreenAction::SwitchTo(Screen::Dashboard)));
    }

    #[test]
    fn key_2_jumps_to_local() {
        let mut app = fixture_app();
        let mut pending_g = false;
        let action = handle_key(&mut app, KeyCode::Char('2'), &mut pending_g);
        assert!(matches!(action, ScreenAction::SwitchTo(Screen::Local)));
    }

    #[test]
    fn key_3_jumps_to_remote() {
        let mut app = fixture_app();
        let mut pending_g = false;
        let action = handle_key(&mut app, KeyCode::Char('3'), &mut pending_g);
        assert!(matches!(action, ScreenAction::SwitchTo(Screen::Remote)));
    }

    #[test]
    fn key_comma_jumps_to_settings() {
        let mut app = fixture_app();
        let mut pending_g = false;
        let action = handle_key(&mut app, KeyCode::Char(','), &mut pending_g);
        assert!(matches!(action, ScreenAction::SwitchTo(Screen::Settings)));
    }

    #[test]
    fn key_s_jumps_to_search_from_dashboard() {
        let mut app = fixture_app();
        app.current_screen = Screen::Dashboard;
        let mut pending_g = false;
        let action = handle_key(&mut app, KeyCode::Char('s'), &mut pending_g);
        assert!(matches!(action, ScreenAction::SwitchTo(Screen::Search)));
    }
}
