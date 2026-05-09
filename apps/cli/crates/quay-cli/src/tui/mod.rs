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
                        if *s == Screen::Search || *s == Screen::Browse {
                            // Browse and Search share the same fetched skill list
                            // (browse filters it by selected remote in its preview).
                            screens::search::ensure_loaded(app);
                        }
                        if *s == Screen::Browse {
                            // Populate `app.browse` items + initial selection so
                            // key handlers can read it (render's local clone is
                            // not enough for the [a] install path).
                            screens::browse::ensure_loaded_into_app(app);
                        }
                        if *s == Screen::CreatePush {
                            // Reset the create/push state machine to a fresh form.
                            app.create_push = screens::create_push::CreatePushState::Form(
                                screens::create_push::build_create_form_from_app(app),
                            );
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
            if app.settings.remotes.testing_idx.is_some() {
                app.settings.remotes.spinner.advance();
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
        BlockingAction::Add { skill, remote } => {
            // Run `quay add` against the configured remote, then refresh
            // both the lockfile (so `quay list` and Dashboard `Installed`
            // panel see the new entry) and the local-skills scan (so the
            // badge graduates from `local`/`pushed-direct` to `installed`).
            use quay_core::{Config, GithubRawFetcher, SkillManager};
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
            let branch = std::env::var("QUAY_GITHUB_BRANCH").unwrap_or_else(|_| "main".into());
            let f = GithubRawFetcher::new(branch);
            let mgr = SkillManager::new(&cfg, &f, &f, app.project_root.clone());
            match mgr.add(&skill, remote.as_deref()) {
                Ok(locked) => {
                    if let Ok(lock) = quay_core::Lockfile::load_or_default(
                        &app.project_root.join(".quay/lockfile.json"),
                    ) {
                        app.lock = lock;
                    }
                    app.reload_local_skills();
                    app.set_status(format!(
                        "installed {} v{} from {}",
                        skill, locked.version, locked.remote
                    ));
                }
                Err(e) => {
                    app.set_status(format!("install failed: {e}"));
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
                None, // push_mode: use remote default
                None, // profile
                &app.project_root,
                app.user_config_path.as_deref(),
            );
            match result {
                Ok(outcome) => {
                    app.create_push = screens::create_push::CreatePushState::Done(outcome);
                    // Refresh local-skills badges so Dashboard reflects the new
                    // push-log entry without the user having to press [r].
                    app.reload_local_skills();
                }
                Err(e) => {
                    // Move the current Pushing state into a Failed wrapper.
                    // We need to extract the current state to use as the "prior" state.
                    // The Pushing state is already set; we wrap it in Failed.
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

    // When a text input is focused (Onboarding form, Create/Push form, or a
    // Settings add/edit modal), every keystroke must reach the form verbatim.
    // Otherwise typing `evgenii` is impossible — `g` triggers the chord
    // prefix, `p` opens the profile switcher, `q` quits, etc.
    //
    // Esc is intentionally NOT special-cased here — the focused screen's
    // own handler treats Esc as form-cancel / back, which is the same
    // behaviour we want.
    if app.has_focused_text_input() {
        *pending_g = false;
        return match app.current_screen {
            Screen::Onboarding => screens::onboarding::handle_key(app, code),
            Screen::CreatePush => screens::create_push::handle_key(app, code),
            Screen::Settings => screens::settings::handle_key(app, code),
            // The matches! in has_focused_text_input never returns true
            // for these — but the compiler needs an exhaustive match.
            Screen::Dashboard => screens::dashboard::handle_key(app, code),
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
    use crossterm::event::KeyCode as KCode;
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

    /// `onboarded = true` with no profiles still routes to onboarding.
    ///
    /// This was the bug: the old gate used `!onboarded && profiles.is_empty()`,
    /// so a user who hit "skip" once (setting `onboarded=true`, no profiles)
    /// was permanently locked out of profile creation. The new gate uses
    /// `profiles.is_empty()` alone.
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
