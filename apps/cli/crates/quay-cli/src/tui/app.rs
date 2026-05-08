//! Application state shared across all TUI screens.

use crate::config_io;
use crate::tui::screens::browse::BrowseState;
use crate::tui::screens::installed::InstalledState;
use crate::tui::screens::onboarding::OnboardingState;
use crate::tui::screens::search::SearchState;
use quay_core::{BumpKind, Config, Lockfile, UserConfigFile};
use ratatui::widgets::{Block, Borders};
use std::path::PathBuf;
use tui_textarea::TextArea;

/// Overlay modal currently displayed over the main screen.
#[derive(Debug)]
pub enum ModalState {
    /// Profile switcher — opened by `p`, closed by `Esc` or `Enter`.
    ProfileSwitcher(crate::tui::screens::modal_profile_switcher::SwitcherState),
}

/// A deferred blocking action that the event loop runs after the next render.
///
/// This ensures the spinner (or other in-progress indicator) paints at least
/// once before the multi-second blocking call freezes the event loop.
#[derive(Debug)]
pub enum BlockingAction {
    /// Push a skill to a remote hub via PR.
    Push {
        skill: String,
        remote: Option<String>,
        bump: BumpKind,
    },
}

/// Top-level screen selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Dashboard,
    Browse,
    Search,
    Installed,
    Settings,
    CreatePush,
    /// First-run onboarding — shown when no user config exists or
    /// `meta.onboarded == false && profiles.is_empty()`.
    Onboarding,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SettingsTab {
    #[default]
    Profiles,
    Remotes,
    Install,
}

#[derive(Debug, Default)]
pub struct SettingsState {
    pub tab: SettingsTab,
    pub profiles: crate::tui::screens::settings::profiles::ProfilesState,
    pub remotes: crate::tui::screens::settings::remotes::RemotesState,
    pub install: crate::tui::screens::settings::install::InstallState,
}

#[derive(Debug)]
pub enum ScreenAction {
    Stay,
    SwitchTo(Screen),
    Quit,
}

/// Returns `true` when the onboarding screen should be shown instead of the
/// dashboard.  Fires for brand-new installs (no config file, or empty config)
/// where `meta.onboarded` is false and no profiles exist.
pub fn should_show_onboarding(file: &UserConfigFile) -> bool {
    !file.meta.onboarded && file.profiles.is_empty()
}

/// Internal helper used by [`App::new`].
///
/// When no user config path is given (`None`), falls back to Dashboard — the
/// caller has not configured a config file location so we cannot write
/// onboarding output anywhere.  When a path is given but the file is missing
/// or empty, onboarding fires.
fn decide_initial_screen_from_file(
    user_config_path: Option<&std::path::Path>,
    result: Result<UserConfigFile, quay_core::QuayError>,
) -> Screen {
    // No config path configured: skip onboarding (we have nowhere to write).
    if user_config_path.is_none() {
        return Screen::Dashboard;
    }
    match result {
        Ok(file) if should_show_onboarding(&file) => Screen::Onboarding,
        Ok(_) => Screen::Dashboard,
        Err(_) => Screen::Onboarding,
    }
}

pub struct App {
    pub cfg: Config,
    pub lock: Lockfile,
    pub project_root: PathBuf,
    pub user_config_path: Option<PathBuf>,
    pub current_screen: Screen,
    pub status_message: Option<String>,
    pub should_quit: bool,
    pub browse: BrowseState,
    pub search: SearchState,
    pub installed: InstalledState,
    pub search_textarea: TextArea<'static>,
    pub settings: SettingsState,
    /// Active overlay modal, if any. When `Some`, the modal intercepts all key events.
    pub modal: Option<ModalState>,
    /// Create/Push screen state machine.
    pub create_push: crate::tui::screens::create_push::CreatePushState,
    /// Onboarding screen state machine.
    pub onboarding: OnboardingState,
    /// A blocking action deferred until after the next render cycle.
    ///
    /// The event loop checks this after each `terminal.draw()` call: if it is
    /// `Some`, the action is taken out and executed synchronously before the
    /// next `event::poll()`.  This guarantees that any state change (e.g.
    /// transitioning to `Pushing`) is painted once before the blocking call
    /// freezes the event loop.
    pub next_blocking: Option<BlockingAction>,
}

impl App {
    pub fn new(
        cfg: Config,
        lock: Lockfile,
        project_root: PathBuf,
        user_config_path: Option<PathBuf>,
    ) -> Self {
        let mut search_textarea = TextArea::default();
        search_textarea.set_block(Block::default().borders(Borders::ALL).title(" search "));
        let create_push = crate::tui::screens::create_push::CreatePushState::Form(
            crate::tui::screens::create_push::FormFields::from_config_remotes(&cfg),
        );
        let initial_screen = decide_initial_screen_from_file(
            user_config_path.as_deref(),
            config_io::read_user_file(user_config_path.as_deref()),
        );
        Self {
            cfg,
            lock,
            project_root,
            user_config_path,
            current_screen: initial_screen,
            status_message: None,
            should_quit: false,
            browse: BrowseState::default(),
            search: SearchState::default(),
            installed: InstalledState::default(),
            search_textarea,
            settings: SettingsState::default(),
            modal: None,
            create_push,
            onboarding: OnboardingState::default(),
            next_blocking: None,
        }
    }

    pub fn switch_to(&mut self, screen: Screen) {
        self.current_screen = screen;
        self.status_message = None;
    }

    pub fn set_status(&mut self, msg: impl Into<String>) {
        self.status_message = Some(msg.into());
    }

    /// Queue a [`BlockingAction`] to be executed after the next render.
    ///
    /// Only one action can be queued at a time.  Calling this while another
    /// action is pending silently overwrites the pending action — callers must
    /// not depend on ordering.
    pub fn defer_blocking_action(&mut self, action: BlockingAction) {
        self.next_blocking = Some(action);
    }
}
