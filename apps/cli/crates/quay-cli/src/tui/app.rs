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
    /// Install a skill from a remote hub into `.agents/skills/`.
    Add {
        skill: String,
        remote: Option<String>,
    },
    /// Probe a remote for reachability, auth, and registry presence.
    TestConnection {
        url: String,
        kind: Option<quay_core::ProviderKind>,
        remote_idx: usize,
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
/// dashboard.
///
/// Driven by `profiles.is_empty()` alone — `meta.onboarded` exists only to
/// suppress repeated skip prompts within onboarding itself, not as a hard gate.
/// This means a user who skipped onboarding once (writing `onboarded = true`
/// with no profiles) still gets onboarding on the next launch — exactly the
/// recovery path the older two-condition gate was missing.
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
    /// Local skills discovered under `.agents/skills/` at startup.
    pub local_skills: Vec<quay_core::scanner::LocalSkill>,
    /// Index of the currently highlighted row in the Local skills panel.
    pub local_selected: usize,
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
        let remotes: Vec<String> = cfg.remotes.keys().cloned().collect();
        let create_push = crate::tui::screens::create_push::CreatePushState::Form(
            crate::tui::screens::create_push::build_create_form(&remotes),
        );
        let initial_screen = decide_initial_screen_from_file(
            user_config_path.as_deref(),
            config_io::read_user_file(user_config_path.as_deref()),
        );
        let local_skills = Self::scan_local(&project_root);
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
            local_skills,
            local_selected: 0,
        }
    }

    /// Scan local skills under `<project_root>/.agents/skills/`.
    fn scan_local(project_root: &std::path::Path) -> Vec<quay_core::scanner::LocalSkill> {
        let lockfile = quay_core::lockfile::Lockfile::load_or_default(
            &project_root.join(".quay/lockfile.json"),
        )
        .unwrap_or_default();
        let push_log = quay_core::push_log::PushLog::load(project_root).unwrap_or_default();
        let scan_root = project_root.join(".agents/skills");
        quay_core::scanner::scan_local_skills(&[scan_root], &lockfile, &push_log)
    }

    /// Re-run the local-skills scan (used by Dashboard `[r]` rescan and after a push).
    pub fn reload_local_skills(&mut self) {
        self.local_skills = Self::scan_local(&self.project_root);
        if self.local_selected >= self.local_skills.len() {
            self.local_selected = self.local_skills.len().saturating_sub(1);
        }
    }

    /// Returns `true` when the focused screen has a text input that should
    /// receive every keystroke verbatim (no global hotkey interception).
    ///
    /// Without this, typing `evgenii` in a profile-name field is impossible:
    /// the global `g`-chord prefix and single-letter screen jumps eat
    /// characters before the form sees them.
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
    ///
    /// Only one action can be queued at a time.  Calling this while another
    /// action is pending silently overwrites the pending action — callers must
    /// not depend on ordering.
    pub fn defer_blocking_action(&mut self, action: BlockingAction) {
        self.next_blocking = Some(action);
    }

    /// Route a bracketed-paste string to the focused form-bearing screen.
    ///
    /// List screens (Dashboard, Browse, Search, Installed) silently drop the
    /// paste.  Form-bearing screens (Onboarding, CreatePush, Settings) forward
    /// the string to whichever text field currently has focus.
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
            // List screens have no text inputs; paste is silently ignored.
            Screen::Dashboard | Screen::Browse | Screen::Search | Screen::Installed => {}
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
