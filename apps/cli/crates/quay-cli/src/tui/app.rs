//! Application state shared across all TUI screens.

use crate::config_io;
use crate::tui::screens::browse::BrowseState;
use crate::tui::screens::installed::InstalledState;
use crate::tui::screens::local::LocalState;
use crate::tui::screens::onboarding::OnboardingState;
use crate::tui::screens::remote::RemoteState;
use crate::tui::screens::search::SearchState;
use quay_core::{BumpKind, Config, UserConfigFile};
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
    /// Install a skill from a remote hub into `.agents/skills/`.
    Add {
        skill: String,
        remote: Option<String>,
        /// If true, overwrite an existing local copy.
        force: bool,
    },
    /// Probe a remote for reachability, auth, and registry presence.
    TestConnection {
        url: String,
        kind: Option<quay_core::ProviderKind>,
        remote_idx: usize,
    },
    /// Fetch (shallow-clone) a remote registry and populate `app.remote.rows`.
    FetchRegistry {
        /// The configured remote name to fetch.
        remote_name: String,
    },
    /// Clone the harbor and run full reconcile for a colliding skill.
    ///
    /// Produces a [`ReconcileReport`] that is surfaced as a
    /// `RemoteModal::Reconcile` on completion.
    Reconcile {
        /// Skill name (relative path under `skills/`).
        skill: String,
        /// The configured remote name to clone from.
        remote: Option<String>,
    },
}

/// Top-level screen selection.
///
/// Plan 10 layout:
///   `[1]` Dashboard — summary panels
///   `[2]` Local     — scan_local across all mirror roots
///   `[3]` Remote    — browse a remote's registry.json
///   `[s]` Search    — live filter across local + remote
///   `[,]` Settings  — profiles / remotes / install
///   `[q]` Quit
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Dashboard,
    /// NEW (Plan 10): replaces the old Browse + Installed screens.
    Local,
    /// NEW (Plan 10): replaces the old Browse-Remote panel.
    Remote,
    Search,
    Settings,
    /// Create/Push — still reachable via `[u]` from Local.
    CreatePush,
    /// First-run onboarding — shown when no user config exists or
    /// `meta.onboarded == false && profiles.is_empty()`.
    Onboarding,
    // Legacy variants kept temporarily so existing tests still compile.
    // Removed from navigation; will be deleted when old screens are fully gone.
    Browse,
    Installed,
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
/// dashboard.
///
/// Driven by `profiles.is_empty()` alone — `meta.onboarded` exists only to
/// suppress repeated skip prompts within onboarding itself, not as a hard gate.
pub fn should_show_onboarding(file: &UserConfigFile) -> bool {
    file.profiles.is_empty()
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
    pub project_root: PathBuf,
    pub user_config_path: Option<PathBuf>,
    pub current_screen: Screen,
    pub status_message: Option<String>,
    pub should_quit: bool,
    // --- Legacy screen state (kept while old screens remain) ---
    pub browse: BrowseState,
    pub installed: InstalledState,
    // -----------------------------------------------------------
    pub search: SearchState,
    pub search_textarea: TextArea<'static>,
    pub settings: SettingsState,
    /// Active overlay modal, if any. When `Some`, the modal intercepts all key events.
    pub modal: Option<ModalState>,
    /// Create/Push screen state machine (also used by Local [u]/[U]).
    pub create_push: crate::tui::screens::create_push::CreatePushState,
    /// Onboarding screen state machine.
    pub onboarding: OnboardingState,
    /// A blocking action deferred until after the next render cycle.
    pub next_blocking: Option<BlockingAction>,
    /// When `true`, the event loop must NOT reset `create_push` on the next
    /// `SwitchTo(Screen::CreatePush)` transition.  Set by Local `[u]`/`[U]`
    /// which pre-populate the state before switching screens.
    pub push_form_ready: bool,
    /// Local skills discovered under all mirror roots at startup.
    pub local_skills: Vec<quay_core::scanner::LocalSkill>,
    /// Index of the currently highlighted row in the Local skills panel (Dashboard legacy).
    pub local_selected: usize,
    /// State for the Local screen ([2]).
    pub local: LocalState,
    /// State for the Remote screen ([3]).
    pub remote: RemoteState,
}

impl App {
    pub fn new(cfg: Config, project_root: PathBuf, user_config_path: Option<PathBuf>) -> Self {
        let mut search_textarea = TextArea::default();
        search_textarea.set_block(Block::default().borders(Borders::ALL).title(" search "));
        let remotes: Vec<String> = cfg.remotes.keys().cloned().collect();
        let create_push = crate::tui::screens::create_push::CreatePushState::Form(
            crate::tui::screens::create_push::build_create_form(&remotes),
        );
        let initial_screen = decide_initial_screen_from_file(
            user_config_path.as_deref(),
            config_io::read_user_file(user_config_path.as_deref()),
        );
        let config_dir_for_scan = user_config_path.as_deref().and_then(|p| p.parent());
        let local_skills = Self::scan_local(&project_root, config_dir_for_scan);
        Self {
            cfg,
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
            push_form_ready: false,
            local_skills,
            local_selected: 0,
            local: LocalState::default(),
            remote: RemoteState::default(),
        }
    }

    /// Scan local skills under all four mirror roots.
    fn scan_local(
        project_root: &std::path::Path,
        config_dir: Option<&std::path::Path>,
    ) -> Vec<quay_core::scanner::LocalSkill> {
        let push_log = quay_core::push_log::PushLog::load(
            config_dir.unwrap_or(project_root),
            Some(project_root),
        )
        .unwrap_or_default();
        quay_core::scanner::scan_local(project_root, &push_log)
    }

    /// Re-run the local-skills scan (used by Local `[r]` rescan and after a push).
    pub fn reload_local_skills(&mut self) {
        let config_dir = self.user_config_path.as_deref().and_then(|p| p.parent());
        self.local_skills = Self::scan_local(&self.project_root, config_dir);
        if self.local_selected >= self.local_skills.len() {
            self.local_selected = self.local_skills.len().saturating_sub(1);
        }
    }

    /// Returns `true` when the focused screen has a text input that should
    /// receive every keystroke verbatim (no global hotkey interception).
    pub fn has_focused_text_input(&self) -> bool {
        use crate::tui::screens::create_push::CreatePushState;
        use crate::tui::screens::onboarding::OnboardingState;

        match self.current_screen {
            Screen::Onboarding => matches!(
                self.onboarding,
                OnboardingState::Profile { .. } | OnboardingState::Remote { .. }
            ),
            Screen::CreatePush => matches!(self.create_push, CreatePushState::Form(_)),
            Screen::Settings => match self.settings.tab {
                SettingsTab::Profiles => crate::tui::screens::settings::profiles::has_active_modal(
                    &self.settings.profiles,
                ),
                SettingsTab::Remotes => {
                    crate::tui::screens::settings::remotes::has_active_modal(&self.settings.remotes)
                }
                SettingsTab::Install => false,
            },
            _ => false,
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
    pub fn defer_blocking_action(&mut self, action: BlockingAction) {
        self.next_blocking = Some(action);
    }

    /// Route a bracketed-paste string to the focused form-bearing screen.
    pub fn handle_paste(&mut self, s: &str) {
        match self.current_screen {
            Screen::Onboarding => {
                crate::tui::screens::onboarding::handle_paste(&mut self.onboarding, s);
            }
            Screen::CreatePush => {
                crate::tui::screens::create_push::handle_paste(&mut self.create_push, s);
            }
            Screen::Settings => {
                crate::tui::screens::settings::handle_paste(&mut self.settings, s);
            }
            // Screens without text inputs silently drop paste.
            Screen::Dashboard
            | Screen::Local
            | Screen::Remote
            | Screen::Browse
            | Screen::Search
            | Screen::Installed => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quay_core::{MetaSection, ProfileFile, UserConfigFile};
    use std::collections::BTreeMap;

    #[test]
    fn onboarding_due_when_profiles_empty_even_if_onboarded_true() {
        let file = UserConfigFile {
            meta: MetaSection { onboarded: true },
            profiles: BTreeMap::new(),
            ..Default::default()
        };
        assert!(
            should_show_onboarding(&file),
            "gate should fire when no profiles exist, regardless of onboarded flag"
        );
    }

    #[test]
    fn onboarding_not_due_when_profiles_present() {
        let mut profiles = BTreeMap::new();
        profiles.insert("p".to_string(), ProfileFile::default());
        let file = UserConfigFile {
            meta: MetaSection { onboarded: true },
            profiles,
            ..Default::default()
        };
        assert!(!should_show_onboarding(&file));
    }
}
