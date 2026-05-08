//! TUI entry point and event loop.

pub mod app;
pub mod editor;
pub mod screens;
pub mod theme;
pub mod widgets;

use crate::commands;
use crate::config_io;
use crate::tui::app::{App, BlockingAction, Screen, ScreenAction};
use crate::tui::screens::onboarding::OnboardingState;
use crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode};
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
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = event_loop(&mut terminal, &mut app);

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
            if let Event::Key(key) = event::read()? {
                let action = handle_key(app, key.code, &mut pending_g);
                if let ScreenAction::SwitchTo(s) = &action {
                    app.switch_to(*s);
                    if *s == Screen::Search {
                        screens::search::ensure_loaded(app);
                    }
                    if *s == Screen::CreatePush {
                        // Reset the create/push state machine to a fresh form.
                        app.create_push = screens::create_push::CreatePushState::Form(
                            screens::create_push::FormFields::from_app(app),
                        );
                    }
                } else if let ScreenAction::Quit = &action {
                    app.should_quit = true;
                }
            }
        }
        if last_tick.elapsed() >= tick_rate {
            // Tick: advance any animated state.
            app.create_push.tick();
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
        BlockingAction::Push {
            skill,
            remote,
            bump,
        } => {
            let result = commands::push::push_skill(
                &skill,
                remote.as_deref(),
                bump,
                None,
                &app.project_root,
                app.user_config_path.as_deref(),
            );
            match result {
                Ok(outcome) => {
                    app.create_push = screens::create_push::CreatePushState::Done(outcome);
                }
                Err(e) => {
                    // Move the current Pushing state into a Failed wrapper.
                    // We need to extract the current state to use as the "prior" state.
                    // The Pushing state is already set; we wrap it in Failed.
                    let placeholder = screens::create_push::CreatePushState::Form(
                        screens::create_push::FormFields::with_remotes(&[]),
                    );
                    let pushing_state = std::mem::replace(&mut app.create_push, placeholder);
                    app.create_push = screens::create_push::CreatePushState::Failed {
                        state: Box::new(pushing_state),
                        message: e.to_string(),
                    };
                }
            }
        }
    }
}

pub fn handle_key(app: &mut App, code: KeyCode, pending_g: &mut bool) -> ScreenAction {
    if app.modal.is_some() {
        screens::modal_profile_switcher::handle_key(app, code);
        return ScreenAction::Stay;
    }

    // Handle the `g`-prefix chord table.
    if *pending_g {
        *pending_g = false;
        let action = match code {
            KeyCode::Char('d') => ScreenAction::SwitchTo(Screen::Dashboard),
            KeyCode::Char('b') => ScreenAction::SwitchTo(Screen::Browse),
            KeyCode::Char('s') => ScreenAction::SwitchTo(Screen::Settings),
            KeyCode::Char('i') => ScreenAction::SwitchTo(Screen::Installed),
            KeyCode::Char('c') => ScreenAction::SwitchTo(Screen::CreatePush),
            _ => ScreenAction::Stay,
        };
        return action;
    }

    // Global keys come first.
    match code {
        KeyCode::Char('q') => return ScreenAction::Quit,
        KeyCode::Char('1') => return ScreenAction::SwitchTo(Screen::Dashboard),
        KeyCode::Char('2') => return ScreenAction::SwitchTo(Screen::Browse),
        KeyCode::Char('3') => return ScreenAction::SwitchTo(Screen::Search),
        KeyCode::Char('4') => return ScreenAction::SwitchTo(Screen::Installed),
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
    use quay_core::{Config, Lockfile};

    fn fixture_app() -> App {
        App::new(
            Config::default(),
            Lockfile::default(),
            std::path::PathBuf::from("/tmp"),
            None,
        )
    }

    // -----------------------------------------------------------------------
    // Task 12 — startup gate tests
    // -----------------------------------------------------------------------

    #[test]
    fn onboarding_when_no_config_file() {
        let dir = assert_fs::TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        // File does not exist — gate should route to onboarding.
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
        // Empty file: default meta (onboarded=false) + no profiles.
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

    #[test]
    fn dashboard_when_skipped_marker_present() {
        let dir = assert_fs::TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[meta]\nonboarded = true\n").unwrap();
        let initial = decide_initial_screen(Some(&path));
        assert!(
            matches!(initial, Screen::Dashboard),
            "expected Dashboard, got {:?}",
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
        // First key: 'g' — sets the pending flag.
        let action = handle_key(&mut app, KeyCode::Char('g'), &mut pending_g);
        assert!(matches!(action, ScreenAction::Stay));
        assert!(pending_g, "pending_g should be true after 'g'");
        // Second key: 'c' — resolves the chord.
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
}
