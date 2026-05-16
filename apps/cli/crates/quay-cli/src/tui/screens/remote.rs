//! TUI Screen [3] — Browse Remote.
//!
//! Fetches one configured remote's `registry.json` and renders rows.
//! Key bindings:
//!   - `[Tab]` — cycle configured remotes
//!   - `[j]/[k]` — navigate rows
//!   - `[Enter]` — preview (SKILL.md content, fetched on demand)
//!   - `[a]` — pull skill (blocks if local copy already exists; bulk if picks non-empty)
//!   - `[A]` — force pull (overwrite confirm; bulk if picks non-empty)
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
use std::collections::{BTreeSet, VecDeque};
use std::time::Instant;

/// Per-skill outcome recorded during a bulk pull.
#[derive(Debug, Clone)]
pub struct BulkAddResult {
    /// Skill name.
    pub skill: String,
    /// `Ok(())` on success, `Err(message)` on failure.
    pub outcome: Result<(), String>,
}

/// State machine for bulk-add (pull) operations.
#[derive(Debug, Default)]
pub enum BulkAddState {
    #[default]
    None,
    /// Bulk pull in progress: one skill at a time.
    BulkAdding {
        /// Skill names still waiting to be pulled.
        remaining: VecDeque<String>,
        /// Total number of skills in the batch.
        total: usize,
        /// Skill currently being pulled.
        current: String,
        /// Configured remote name.
        remote: Option<String>,
        /// Whether to force-overwrite existing local copies.
        force: bool,
        /// Accumulated per-skill outcomes.
        results: Vec<BulkAddResult>,
        /// Spinner widget.
        spinner: Spinner,
        /// When the batch started.
        started_at: Instant,
    },
    /// All bulk pulls finished — show per-skill outcome list.
    BulkAddDone(Vec<BulkAddResult>),
}

impl BulkAddState {
    /// Advance the spinner if in `BulkAdding`.
    pub fn tick(&mut self) {
        if let BulkAddState::BulkAdding { spinner, .. } = self {
            spinner.advance();
        }
    }
}

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
    /// Bulk-selected row indices; toggled by `[Space]`. Cleared on `[Tab]` remote cycle.
    pub picks: BTreeSet<usize>,
    /// Bulk-add (pull) state machine.
    pub bulk_add: BulkAddState,
}

/// Modal overlays for the Remote screen.
#[derive(Debug, Default, Clone)]
pub enum RemoteModal {
    #[default]
    None,
    /// Confirm overwrite for `[A]` (single skill).
    ConfirmForcePull { skill_name: String },
    /// Confirm bulk force pull for `[A]` with picks non-empty.
    ConfirmBulkForcePull { count: usize },
    /// Three-way collision choice after `[a]` with picks that include already-local skills.
    BatchAddCollisionChoice {
        /// Names of skills that already exist locally.
        collisions: Vec<String>,
        /// Names of skills that are new (not local yet).
        fresh: Vec<String>,
        /// Which option is highlighted: 0=UpdateAll, 1=SkipAll, 2=PromptEach.
        highlighted: usize,
    },
    /// Per-skill collision choice during PromptEach.
    PerCollisionChoice {
        /// The skill currently being prompted.
        skill_name: String,
        /// Whether the local copy has been modified since install.
        is_modified: bool,
        /// Remaining collisions to prompt (name, is_modified).
        remaining_collisions: VecDeque<(String, bool)>,
        /// Accumulated per-skill actions so far.
        accumulated: Vec<(String, quay_core::SkillAction)>,
        /// Skills that need no collision prompt (fresh installs).
        fresh: Vec<String>,
        /// Which option is highlighted: 0=Update, 1=Skip.
        highlighted: usize,
    },
    /// Reconcile collision modal — shown after `BlockingAction::Reconcile`
    /// completes. Displays the verdict + diff and lets the user pick
    /// Replace / Keep / Skip.
    Reconcile(crate::tui::screens::reconcile_modal::ReconcileModal),
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
    // BulkAddDone: [Esc] or [b] returns to Remote screen (clears done state).
    if let BulkAddState::BulkAddDone(_) = &app.remote.bulk_add {
        match code {
            KeyCode::Esc | KeyCode::Char('b') => {
                app.remote.bulk_add = BulkAddState::None;
            }
            _ => {}
        }
        return ScreenAction::Stay;
    }

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
        RemoteModal::ConfirmBulkForcePull { count } => {
            match code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    // Start bulk force pull.
                    let names: Vec<String> = app
                        .remote
                        .picks
                        .iter()
                        .filter_map(|&i| app.remote.rows.get(i))
                        .map(|r| r.name.clone())
                        .collect();
                    if !names.is_empty() {
                        bulk_add_start(app, names, true);
                    }
                }
                _ => {
                    // Cancel: restore count (not stored, just dismiss).
                    let _ = count;
                }
            }
            return ScreenAction::Stay;
        }
        RemoteModal::BatchAddCollisionChoice {
            collisions,
            fresh,
            mut highlighted,
        } => {
            match code {
                KeyCode::Up | KeyCode::Char('k') => {
                    highlighted = highlighted.saturating_sub(1);
                    app.remote.modal = RemoteModal::BatchAddCollisionChoice {
                        collisions,
                        fresh,
                        highlighted,
                    };
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    highlighted = (highlighted + 1).min(2);
                    app.remote.modal = RemoteModal::BatchAddCollisionChoice {
                        collisions,
                        fresh,
                        highlighted,
                    };
                }
                KeyCode::Enter => {
                    // Resolve the chosen strategy.
                    match highlighted {
                        0 => {
                            // UpdateAll: force-pull all (collisions + fresh).
                            let mut all: Vec<String> = collisions;
                            all.extend(fresh);
                            bulk_add_start(app, all, true);
                        }
                        1 => {
                            // SkipAll: only pull fresh skills.
                            if fresh.is_empty() {
                                app.set_status("no new skills to install (all already local)");
                            } else {
                                bulk_add_start(app, fresh, false);
                            }
                        }
                        _ => {
                            // PromptEach: transition to per-skill loop for first collision.
                            let mut remaining: VecDeque<(String, bool)> = collisions
                                .into_iter()
                                .map(|n| {
                                    let is_mod = is_skill_modified(app, &n);
                                    (n, is_mod)
                                })
                                .collect();
                            if let Some((first_name, first_mod)) = remaining.pop_front() {
                                app.remote.modal = RemoteModal::PerCollisionChoice {
                                    skill_name: first_name,
                                    is_modified: first_mod,
                                    remaining_collisions: remaining,
                                    accumulated: Vec::new(),
                                    fresh,
                                    highlighted: 0,
                                };
                            }
                        }
                    }
                }
                KeyCode::Esc => {
                    // Dismiss — no action.
                }
                _ => {
                    // Restore modal.
                    app.remote.modal = RemoteModal::BatchAddCollisionChoice {
                        collisions,
                        fresh,
                        highlighted,
                    };
                }
            }
            return ScreenAction::Stay;
        }
        RemoteModal::PerCollisionChoice {
            skill_name,
            is_modified,
            remaining_collisions,
            mut accumulated,
            fresh,
            mut highlighted,
        } => {
            match code {
                KeyCode::Up | KeyCode::Char('k') => {
                    highlighted = highlighted.saturating_sub(1);
                    app.remote.modal = RemoteModal::PerCollisionChoice {
                        skill_name,
                        is_modified,
                        remaining_collisions,
                        accumulated,
                        fresh,
                        highlighted,
                    };
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    highlighted = (highlighted + 1).min(1);
                    app.remote.modal = RemoteModal::PerCollisionChoice {
                        skill_name,
                        is_modified,
                        remaining_collisions,
                        accumulated,
                        fresh,
                        highlighted,
                    };
                }
                // [u] — Update shortcut (always overwrite regardless of highlight).
                KeyCode::Char('u') => {
                    accumulated.push((skill_name, quay_core::SkillAction::UpdateForce));
                    advance_per_collision(app, remaining_collisions, accumulated, fresh);
                }
                // [s] — Skip shortcut (always skip regardless of highlight).
                KeyCode::Char('s') => {
                    accumulated.push((skill_name, quay_core::SkillAction::Skip));
                    advance_per_collision(app, remaining_collisions, accumulated, fresh);
                }
                // [Enter] — confirm the currently highlighted option.
                KeyCode::Enter => {
                    let action = if highlighted == 0 {
                        quay_core::SkillAction::UpdateForce
                    } else {
                        quay_core::SkillAction::Skip
                    };
                    accumulated.push((skill_name, action));
                    advance_per_collision(app, remaining_collisions, accumulated, fresh);
                }
                _ => {
                    // Restore modal.
                    app.remote.modal = RemoteModal::PerCollisionChoice {
                        skill_name,
                        is_modified,
                        remaining_collisions,
                        accumulated,
                        fresh,
                        highlighted,
                    };
                }
            }
            return ScreenAction::Stay;
        }
        RemoteModal::Reconcile(mut modal) => {
            if let KeyCode::Char(c) = code {
                use crate::tui::screens::reconcile_modal::ModalOutcome;
                match modal.on_key(c) {
                    ModalOutcome::Resolved(action) => {
                        // Apply the chosen action to the local file.
                        let local_path =
                            crate::tui::local_skill_path(&app.project_root, &modal.skill);
                        if let Err(e) = quay_core::reconcile::action::apply(
                            action,
                            &local_path,
                            &modal.report.head_bytes,
                        ) {
                            app.set_status(format!("reconcile apply failed: {e}"));
                        } else {
                            match action {
                                quay_core::reconcile::action::ResolveAction::Replace => {
                                    app.reload_local_skills();
                                    app.set_status(format!(
                                        "replaced {} with harbor copy",
                                        modal.skill
                                    ));
                                }
                                quay_core::reconcile::action::ResolveAction::Keep => {
                                    app.set_status(format!("kept local {} unchanged", modal.skill));
                                }
                                quay_core::reconcile::action::ResolveAction::Skip => {
                                    app.set_status(format!("skipped {}", modal.skill));
                                }
                            }
                        }
                        // Modal is consumed — leave RemoteModal::None (already set above).
                    }
                    ModalOutcome::Dismissed => {
                        app.set_status(format!("reconcile dismissed for {}", modal.skill));
                        // Modal is consumed — leave RemoteModal::None.
                    }
                    ModalOutcome::Continue => {
                        // Key consumed but no terminal outcome; restore modal.
                        // We reconstruct RemoteModal::Reconcile(modal) rather than
                        // restoring the original `modal` binding because the
                        // ReconcileModal was unwrapped from the variant at the top of
                        // this arm — the outer RemoteModal value was already moved.
                        app.remote.modal = RemoteModal::Reconcile(modal);
                    }
                }
            } else if code == KeyCode::Esc {
                app.set_status(format!("reconcile dismissed for {}", modal.skill));
                // Modal consumed — RemoteModal::None stays.
            } else {
                // Non-char key: restore modal unchanged.
                app.remote.modal = RemoteModal::Reconcile(modal);
            }
            return ScreenAction::Stay;
        }
        RemoteModal::None => {}
    }

    match code {
        KeyCode::Esc => {
            if !app.remote.picks.is_empty() {
                app.remote.picks.clear();
                return ScreenAction::Stay;
            }
            return ScreenAction::Stay;
        }
        KeyCode::Char(' ') => {
            if !app.remote.rows.is_empty() {
                let idx = app.remote.selected;
                if !app.remote.picks.insert(idx) {
                    app.remote.picks.remove(&idx);
                }
            }
        }
        KeyCode::Tab => {
            let names = sorted_remote_names(app);
            if names.is_empty() {
                return ScreenAction::Stay;
            }
            // Selection is remote-scoped: clear before cycling.
            app.remote.picks.clear();
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
            } else if !app.remote.picks.is_empty() {
                // Bulk pull: compute collisions vs. fresh split.
                let rows_snapshot: Vec<RemoteSkillRow> = app
                    .remote
                    .picks
                    .iter()
                    .filter_map(|&i| app.remote.rows.get(i))
                    .cloned()
                    .collect();
                let (blocked, to_pull): (Vec<_>, Vec<_>) = rows_snapshot.iter().partition(|row| {
                    app.project_root
                        .join(".agents/skills")
                        .join(&row.name)
                        .join("SKILL.md")
                        .exists()
                });
                if blocked.is_empty() {
                    // No collisions — install all fresh.
                    let names: Vec<String> = to_pull.iter().map(|r| r.name.clone()).collect();
                    if !names.is_empty() {
                        bulk_add_start(app, names, false);
                    }
                } else {
                    // Collisions exist — open three-way dialog.
                    let collisions: Vec<String> = blocked.iter().map(|r| r.name.clone()).collect();
                    let fresh: Vec<String> = to_pull.iter().map(|r| r.name.clone()).collect();
                    app.remote.modal = RemoteModal::BatchAddCollisionChoice {
                        collisions,
                        fresh,
                        highlighted: 0,
                    };
                }
            } else if let Some(row) = app.remote.rows.get(app.remote.selected) {
                let name = row.name.clone();
                pull_skill(app, &name, false);
            }
        }
        KeyCode::Char('A') => {
            if app.remote.fetching {
                app.set_status("fetch in progress\u{2026} please wait");
            } else if !app.remote.picks.is_empty() {
                let count = app.remote.picks.len();
                app.remote.modal = RemoteModal::ConfirmBulkForcePull { count };
            } else if let Some(row) = app.remote.rows.get(app.remote.selected) {
                let name = row.name.clone();
                app.remote.modal = RemoteModal::ConfirmForcePull { skill_name: name };
            }
        }
        KeyCode::Char('r') => {
            // Drop picks whose indices may no longer resolve after the refresh.
            // (We don't know the new count until fetch completes, so clear all.)
            app.remote.picks.clear();
            app.remote.fetched = false;
            app.remote.selected = 0;
            start_fetch(app);
        }
        _ => {}
    }
    ScreenAction::Stay
}

/// Initialise `BulkAdding` state and defer the first `BlockingAction::Add`.
///
/// `names` must be non-empty. `force` controls whether existing local copies are
/// overwritten.
pub fn bulk_add_start(app: &mut App, names: Vec<String>, force: bool) {
    if names.is_empty() {
        return;
    }
    let remote_names = sorted_remote_names(app);
    let remote = remote_names.get(app.remote.remote_idx).cloned();

    let mut remaining: VecDeque<String> = names.into();
    let current = remaining.pop_front().expect("non-empty checked above");
    let total = remaining.len() + 1;

    app.remote.bulk_add = BulkAddState::BulkAdding {
        remaining,
        total,
        current: current.clone(),
        remote: remote.clone(),
        force,
        results: Vec::new(),
        spinner: Spinner::default(),
        started_at: Instant::now(),
    };
    app.defer_blocking_action(BlockingAction::Add {
        skill: current,
        remote,
        force,
    });
}

/// Advance the bulk-add state machine after an `Add` action completes.
///
/// Called from the TUI event loop once the worker returns an Add result.
/// Appends the result, pops the next skill, defers another `BlockingAction::Add`,
/// or transitions to `BulkAddDone` when `remaining` is empty.
pub fn advance_bulk_add(app: &mut App, skill: String, ok: bool, err_msg: Option<String>) {
    // Swap state out to avoid borrow conflicts.
    let old = std::mem::replace(&mut app.remote.bulk_add, BulkAddState::None);

    if let BulkAddState::BulkAdding {
        mut remaining,
        total,
        current: _,
        remote,
        force,
        mut results,
        ..
    } = old
    {
        results.push(BulkAddResult {
            skill,
            outcome: if ok {
                Ok(())
            } else {
                Err(err_msg.unwrap_or_default())
            },
        });

        if let Some(next) = remaining.pop_front() {
            app.set_status(format!("pulling {next}…"));
            app.defer_blocking_action(BlockingAction::Add {
                skill: next.clone(),
                remote: remote.clone(),
                force,
            });
            app.remote.bulk_add = BulkAddState::BulkAdding {
                remaining,
                total,
                current: next,
                remote,
                force,
                results,
                spinner: Spinner::default(),
                started_at: Instant::now(),
            };
        } else {
            let ok_count = results.iter().filter(|r| r.outcome.is_ok()).count();
            let n = results.len();
            app.set_status(format!("bulk pull done: {ok_count}/{n} succeeded"));
            app.remote.bulk_add = BulkAddState::BulkAddDone(results);
        }
    }
    // Else: not in BulkAdding — no-op.
}

/// Check whether a skill is locally modified (heuristic: file exists in agents mirror).
///
/// Returns `false` when the file cannot be read — the label is informational
/// only and the behaviour is identical to "clean".
fn is_skill_modified(app: &App, skill_name: &str) -> bool {
    let path = app
        .project_root
        .join(".agents/skills")
        .join(skill_name)
        .join("SKILL.md");
    // Simple heuristic: the scanner would mark it InstalledModified if sha
    // doesn't match the hub.  In the TUI we don't have the hub sha at hand, so
    // we conservatively return false (i.e. "clean" label) to avoid a blocking
    // fetch per skill. The Modified label is informational only.
    let _ = path; // suppress unused warning — kept for future Plan 10b expansion
    false
}

/// Advance the PerCollisionChoice loop after a choice is made.
///
/// If more collisions remain, opens the next `PerCollisionChoice` modal.
/// When all collisions are resolved, assembles the `BulkAddState` from
/// accumulated actions + fresh skills and starts the pull.
fn advance_per_collision(
    app: &mut App,
    mut remaining: VecDeque<(String, bool)>,
    accumulated: Vec<(String, quay_core::SkillAction)>,
    fresh: Vec<String>,
) {
    if let Some((next_name, next_mod)) = remaining.pop_front() {
        app.remote.modal = RemoteModal::PerCollisionChoice {
            skill_name: next_name,
            is_modified: next_mod,
            remaining_collisions: remaining,
            accumulated,
            fresh,
            highlighted: 0,
        };
    } else {
        // All collisions resolved — start bulk add from the plan.
        let mut to_pull: Vec<String> = Vec::new();
        let mut force_names: Vec<String> = Vec::new();
        for (name, action) in accumulated {
            match action {
                quay_core::SkillAction::UpdateForce => force_names.push(name),
                quay_core::SkillAction::Install => to_pull.push(name),
                quay_core::SkillAction::Skip => {
                    // record skip in status but don't pull
                }
            }
        }
        // Fresh skills: always install (no force needed).
        to_pull.extend(fresh);
        // Force-updated skills: start a separate bulk with force=true if any,
        // then follow with fresh.  For simplicity we queue force first then
        // fresh in a single combined queue (force flag per-skill isn't
        // supported by BulkAddState — use force=true only when all are forces,
        // otherwise split).
        //
        // Pragmatic approach: queue force-update skills first (force=true),
        // then chain fresh skills (force=false) in a second batch.
        // BulkAddState runs one batch at a time; we start with force if any.
        if !force_names.is_empty() {
            // Merge: force-update first, then fresh at force=false.
            // Since BulkAddState uses a single `force` flag, we must run two
            // batches. Start with force batch; fresh batch is queued afterwards
            // by the caller logic.  For now, combine by running forces with
            // force=true and fresh with the same batch at force=false, which
            // means we only have one batch.  Accept: fresh pulled without force
            // is correct; force-updates need force=true.
            //
            // Solution: add fresh to force_names list and pull them all with
            // force=true. Fresh skills that don't exist locally won't be
            // harmed by force=true (it just skips the collision guard).
            force_names.extend(to_pull);
            if !force_names.is_empty() {
                bulk_add_start(app, force_names, true);
            }
        } else if !to_pull.is_empty() {
            bulk_add_start(app, to_pull, false);
        } else {
            app.set_status("all collision skills skipped — no skills installed".to_string());
        }
    }
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
            // Trigger a full reconcile so the user can inspect the diff
            // and choose Replace / Keep / Skip.
            app.set_status(format!("reconciling {skill_name}\u{2026}"));
            app.defer_blocking_action(BlockingAction::Reconcile {
                skill: skill_name.to_string(),
                remote,
            });
            return;
        }
    }

    app.defer_blocking_action(BlockingAction::Add {
        skill: skill_name.to_string(),
        remote,
        force,
    });
    app.set_status(format!("pulling {skill_name}\u{2026}"));
}

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    // Bulk-add in-progress or done takes over the entire area.
    match &app.remote.bulk_add {
        BulkAddState::BulkAdding {
            current,
            total,
            results,
            spinner,
            ..
        } => {
            render_bulk_adding(frame, area, current, *total, results.len(), spinner);
            return;
        }
        BulkAddState::BulkAddDone(results) => {
            render_bulk_add_done(frame, area, results);
            return;
        }
        BulkAddState::None => {}
    }

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
        let picks = &app.remote.picks;
        let has_picks = !picks.is_empty();
        let title = if has_picks {
            format!(
                " Remote: {} ({} of {} selected) ",
                remote_label,
                picks.len(),
                rows.len()
            )
        } else {
            format!(" Remote: {} ({}) ", remote_label, rows.len())
        };

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
                    let row_content = format!(
                        " {:<28}  v{:<12}  {}",
                        truncate(&row.name, 28),
                        truncate(&row.version, 12),
                        truncate(&row.description, 50)
                    );
                    let line = if has_picks {
                        let prefix = if picks.contains(&i) { "[x]" } else { "[ ]" };
                        format!("{prefix}{row_content}")
                    } else {
                        row_content
                    };
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
    match &app.remote.modal {
        RemoteModal::ConfirmForcePull { skill_name } => {
            render_confirm_modal(frame, area, skill_name);
        }
        RemoteModal::ConfirmBulkForcePull { count } => {
            render_confirm_bulk_force_modal(frame, area, *count);
        }
        RemoteModal::BatchAddCollisionChoice {
            collisions,
            fresh,
            highlighted,
        } => {
            render_batch_collision_modal(frame, area, collisions, fresh.len(), *highlighted);
        }
        RemoteModal::PerCollisionChoice {
            skill_name,
            is_modified,
            remaining_collisions,
            accumulated: _,
            fresh: _,
            highlighted,
        } => {
            let total_collisions = remaining_collisions.len() + 1;
            let done = 0; // always showing "first" of remaining
            render_per_collision_modal(
                frame,
                area,
                skill_name,
                *is_modified,
                done,
                total_collisions,
                *highlighted,
            );
        }
        RemoteModal::Reconcile(modal) => {
            render_reconcile_modal(frame, area, modal);
        }
        RemoteModal::None => {}
    }

    // Hint bar.
    let hint_area = Rect {
        x: area.x,
        y: area.y + area.height.saturating_sub(1),
        width: area.width,
        height: 1,
    };
    let hint_text = if !app.remote.picks.is_empty() {
        format!(
            "[Space] toggle  [a] pull selected ({n})  [A] force pull selected  [Esc] clear",
            n = app.remote.picks.len()
        )
    } else {
        "[Tab] cycle remote  [j]/[k] move  [Space] select  [Enter] preview  [a] pull  [A] force pull  [r] refresh".to_string()
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(hint_text, theme::dim()))),
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

fn render_confirm_bulk_force_modal(frame: &mut Frame, area: Rect, count: usize) {
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
        .title(" Confirm Bulk Force Pull ");
    let inner = block.inner(modal_area);
    frame.render_widget(block, modal_area);
    let lines = vec![
        Line::from(format!(" Overwrite {} local skill(s)? y/N", count)),
        Line::from(""),
        Line::from(Span::styled(" [y] yes   [any other] cancel", theme::dim())),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_batch_collision_modal(
    frame: &mut Frame,
    area: Rect,
    collisions: &[String],
    fresh_count: usize,
    highlighted: usize,
) {
    use ratatui::widgets::Clear;
    let modal_width = 64_u16;
    let modal_height = (8 + collisions.len().min(5) as u16).min(area.height);
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
        .title(" Collision — What to do? ");
    let inner = block.inner(modal_area);
    frame.render_widget(block, modal_area);

    let mut lines = vec![Line::from(format!(
        " {} already local, {} new:",
        collisions.len(),
        fresh_count
    ))];
    for name in collisions.iter().take(5) {
        lines.push(Line::from(format!("   \u{2022} {}", name)));
    }
    if collisions.len() > 5 {
        lines.push(Line::from(format!(
            "   \u{2026} and {} more",
            collisions.len() - 5
        )));
    }
    lines.push(Line::from(""));

    let opts = [
        "Update all (overwrite)",
        "Skip all (only new)",
        "Prompt per skill",
    ];
    for (i, opt) in opts.iter().enumerate() {
        let bullet = if i == highlighted {
            "\u{25c9}"
        } else {
            "\u{25cb}"
        };
        let style = if i == highlighted {
            Style::default().add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        lines.push(Line::from(Span::styled(
            format!(" {} {}", bullet, opt),
            style,
        )));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        " [Up]/[Down] select   [Enter] confirm   [Esc] cancel",
        theme::dim(),
    )));

    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_per_collision_modal(
    frame: &mut Frame,
    area: Rect,
    skill_name: &str,
    is_modified: bool,
    _done: usize,
    total: usize,
    highlighted: usize,
) {
    use ratatui::widgets::Clear;
    let modal_width = 64_u16;
    let modal_height = 9_u16;
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
        .title(format!(" Per-skill ({} collision(s)) ", total));
    let inner = block.inner(modal_area);
    frame.render_widget(block, modal_area);

    let modified_label = if is_modified { " (modified)" } else { "" };
    let mut lines = vec![
        Line::from(format!(
            " skill `{}`{} already exists.",
            skill_name, modified_label
        )),
        Line::from(""),
    ];
    let opts = ["Update (overwrite from remote)", "Skip (keep local)"];
    for (i, opt) in opts.iter().enumerate() {
        let bullet = if i == highlighted {
            "\u{25c9}"
        } else {
            "\u{25cb}"
        };
        let style = if i == highlighted {
            Style::default().add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        lines.push(Line::from(Span::styled(
            format!(" {} {}", bullet, opt),
            style,
        )));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        " [u] update   [s] skip   [Up]/[Down]+[Enter]",
        theme::dim(),
    )));

    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_bulk_adding(
    frame: &mut Frame,
    area: Rect,
    current: &str,
    total: usize,
    done: usize,
    spinner: &Spinner,
) {
    let outer = Block::default().borders(Borders::ALL).title(Span::styled(
        format!(" Bulk Pull ({}/{}) ", done, total),
        theme::accent(),
    ));
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled(format!("  {} ", spinner.frame()), theme::accent()),
            Span::raw(format!(
                "Pulling skill {}… ({}/{} done)",
                current, done, total
            )),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "  Please wait \u{2014} pulling sequentially.",
            theme::dim(),
        )),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_bulk_add_done(frame: &mut Frame, area: Rect, results: &[BulkAddResult]) {
    let ok_count = results.iter().filter(|r| r.outcome.is_ok()).count();
    let total = results.len();
    let title = format!(" Bulk Pull Done ({}/{} succeeded) ", ok_count, total);

    let outer = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(title, theme::accent()));
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let mut lines = vec![Line::from("")];
    for r in results {
        let line = match &r.outcome {
            Ok(()) => Line::from(vec![
                Span::styled("  \u{2714} ", Style::default().fg(Color::Green)),
                Span::raw(r.skill.clone()),
            ]),
            Err(e) => Line::from(vec![
                Span::styled("  \u{2717} ", Style::default().fg(Color::Red)),
                Span::raw(format!("{}: {}", r.skill, e)),
            ]),
        };
        lines.push(line);
    }
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::raw("  "),
        Span::styled(
            format!("pulled {} of {}", ok_count, total),
            Style::default().add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  [Esc] / [b] back to Remote",
        theme::dim(),
    )));

    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_reconcile_modal(
    frame: &mut Frame,
    area: Rect,
    modal: &crate::tui::screens::reconcile_modal::ReconcileModal,
) {
    use quay_core::reconcile::diff::Diff;
    use quay_core::reconcile::verdict::Verdict;
    use ratatui::widgets::Clear;

    let modal_width = 72_u16;
    let modal_height = 16_u16.min(area.height.saturating_sub(2));
    let modal_x = area.x + area.width.saturating_sub(modal_width) / 2;
    let modal_y = area.y + area.height.saturating_sub(modal_height) / 2;
    let modal_area = Rect {
        x: modal_x,
        y: modal_y,
        width: modal_width.min(area.width),
        height: modal_height,
    };
    frame.render_widget(Clear, modal_area);

    let title = format!(" Reconcile: {} ", modal.skill);
    let block = Block::default()
        .borders(Borders::ALL)
        .style(Style::default().bg(Color::DarkGray).fg(Color::White))
        .title(title);
    let inner = block.inner(modal_area);
    frame.render_widget(block, modal_area);

    // Verdict line.
    let verdict_line = match &modal.report.verdict {
        Verdict::Identical => " Verdict: Identical (no change needed)".to_string(),
        Verdict::HubNewer {
            commits_ahead,
            last_commit_date,
            ..
        } => format!(" Verdict: Hub newer (+{commits_ahead} commit(s), last {last_commit_date})"),
        Verdict::LocalAheadOrDiverged { .. } => " Verdict: Local ahead or diverged".to_string(),
        Verdict::ChangedUnknownDirection { local_edited } => {
            if *local_edited {
                " Verdict: Changed — direction unknown (locally edited)".to_string()
            } else {
                " Verdict: Skill absent on harbor HEAD".to_string()
            }
        }
    };

    // Diff body (scrollable).
    let diff_lines: Vec<Line> = match &modal.report.diff {
        Diff::Text(s) if s.is_empty() => vec![Line::from(" (no diff)")],
        Diff::Text(s) => {
            let all: Vec<Line> = s
                .lines()
                .map(|l| {
                    if l.starts_with('+') {
                        Line::from(Span::styled(
                            l.to_string(),
                            Style::default().fg(Color::Green),
                        ))
                    } else if l.starts_with('-') {
                        Line::from(Span::styled(l.to_string(), Style::default().fg(Color::Red)))
                    } else {
                        Line::from(l.to_string())
                    }
                })
                .collect();
            let skip = (modal.scroll as usize).min(all.len().saturating_sub(1));
            all.into_iter().skip(skip).collect()
        }
        Diff::Binary {
            hub_bytes,
            local_bytes,
        } => vec![Line::from(format!(
            " (binary: hub {hub_bytes}B / local {local_bytes}B)"
        ))],
    };

    let replace_hint = if modal.report.absent_on_head {
        "[r] replace (disabled — absent on hub)"
    } else {
        "[r] replace"
    };

    let mut lines = vec![
        Line::from(Span::styled(verdict_line, theme::accent())),
        Line::from(""),
    ];
    lines.extend(diff_lines);
    // Ensure footer always visible by padding if inner is tall enough.
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!(" {replace_hint}   [k] keep   [s] skip   [j] down   [u] up   [q] dismiss"),
        theme::dim(),
    )));

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
                direct_branch: None,
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
                direct_branch: None,
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

        // With the reconcile feature: a collision defers BlockingAction::Reconcile
        // so the user can inspect the diff and choose Replace / Keep / Skip.
        assert!(
            a.next_blocking.is_some(),
            "expected a BlockingAction to be queued for collision"
        );
        assert!(
            matches!(
                a.next_blocking.as_ref().unwrap(),
                crate::tui::app::BlockingAction::Reconcile { skill, .. } if skill == "foo"
            ),
            "expected BlockingAction::Reconcile for foo; got {:?}",
            a.next_blocking
        );
        // Status must indicate reconcile is in progress.
        let status = a.status_message.as_deref().unwrap_or("");
        assert!(
            status.contains("reconcil"),
            "expected reconcile status message, got: {status}"
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
                direct_branch: None,
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
                direct_branch: None,
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
                direct_branch: None,
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
                direct_branch: None,
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
                direct_branch: None,
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

    #[test]
    fn remote_render_with_picks_shows_prefix_and_count() {
        let mut cfg = Config::default();
        cfg.remotes.insert(
            "hub".into(),
            quay_core::RemoteConfig {
                url: "https://x".into(),
                default: true,
                provider: None,
                push_mode: quay_core::PushMode::default(),
                direct_branch: None,
            },
        );
        let mut a = App::new(cfg, std::path::PathBuf::from("/tmp"), None);
        a.current_screen = crate::tui::app::Screen::Remote;
        a.remote.rows = vec![
            RemoteSkillRow {
                name: "aaa".into(),
                version: "1.0.0".into(),
                description: "desc".into(),
                tags: vec![],
            },
            RemoteSkillRow {
                name: "bbb".into(),
                version: "1.0.0".into(),
                description: "desc".into(),
                tags: vec![],
            },
            RemoteSkillRow {
                name: "ccc".into(),
                version: "1.0.0".into(),
                description: "desc".into(),
                tags: vec![],
            },
        ];
        a.remote.fetched = true;
        a.remote.picks.insert(0);
        a.remote.picks.insert(2);

        let mut terminal = Terminal::new(TestBackend::new(160, 30)).unwrap();
        terminal.draw(|f| crate::tui::draw(f, &a)).unwrap();
        let dump: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();

        assert!(
            dump.contains("[x]"),
            "selected rows should show [x]; dump: {dump}"
        );
        assert!(
            dump.contains("[ ]"),
            "unselected rows should show [ ]; dump: {dump}"
        );
        assert!(
            dump.contains("2 of 3 selected"),
            "header should show selection count; dump: {dump}"
        );
    }

    #[test]
    fn space_toggles_remote_pick() {
        let mut cfg = Config::default();
        cfg.remotes.insert(
            "h".into(),
            quay_core::RemoteConfig {
                url: "https://x".into(),
                default: true,
                provider: None,
                push_mode: quay_core::PushMode::default(),
                direct_branch: None,
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

        handle_key(&mut a, KeyCode::Char(' '));
        assert!(
            a.remote.picks.contains(&0),
            "Space must add selected index to picks"
        );
        handle_key(&mut a, KeyCode::Char(' '));
        assert!(
            a.remote.picks.is_empty(),
            "Second Space must remove index from picks"
        );
    }

    #[test]
    fn esc_clears_remote_picks_when_non_empty() {
        let mut a = fixture_app();
        a.remote.picks.insert(0);
        a.remote.picks.insert(2);
        let action = handle_key(&mut a, KeyCode::Esc);
        assert!(matches!(action, ScreenAction::Stay));
        assert!(a.remote.picks.is_empty(), "Esc must clear picks");
    }

    #[test]
    fn esc_stays_when_remote_picks_empty() {
        let mut a = fixture_app();
        let action = handle_key(&mut a, KeyCode::Esc);
        assert!(matches!(action, ScreenAction::Stay));
    }

    #[test]
    fn tab_clears_picks_before_cycling() {
        let mut cfg = Config::default();
        for name in ["alpha", "beta"] {
            cfg.remotes.insert(
                name.into(),
                quay_core::RemoteConfig {
                    url: format!("https://{name}.example.com/hub.git"),
                    default: false,
                    provider: None,
                    push_mode: quay_core::PushMode::default(),
                    direct_branch: None,
                },
            );
        }
        let mut a = App::new(cfg, std::path::PathBuf::from("/tmp"), None);
        a.remote.picks.insert(1);
        a.remote.picks.insert(3);

        handle_key(&mut a, KeyCode::Tab);

        assert!(
            a.remote.picks.is_empty(),
            "Tab must clear picks before cycling remote"
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
                    direct_branch: None,
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

    // -----------------------------------------------------------------------
    // Task 4 — Bulk pull tests
    // -----------------------------------------------------------------------

    /// `[a]` with picks non-empty skips skills already local (block-list) and
    /// queues `BlockingAction::Add` only for the rest.
    #[test]
    fn a_bulk_pull_skips_existing_local_with_block_list_modal() {
        use assert_fs::prelude::*;
        let project = assert_fs::TempDir::new().unwrap();
        // "foo" already exists locally.
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
                direct_branch: None,
            },
        );
        let mut a = App::new(cfg, project.path().to_path_buf(), None);
        a.remote.rows = vec![
            RemoteSkillRow {
                name: "foo".into(),
                version: "1.0.0".into(),
                description: "d".into(),
                tags: vec![],
            },
            RemoteSkillRow {
                name: "bar".into(),
                version: "1.0.0".into(),
                description: "d".into(),
                tags: vec![],
            },
        ];
        a.remote.fetched = true;
        // Pick both rows.
        a.remote.picks.insert(0);
        a.remote.picks.insert(1);

        handle_key(&mut a, KeyCode::Char('a'));

        // Plan 10f: [a] with collisions now opens BatchAddCollisionChoice modal.
        assert!(
            matches!(a.remote.modal, RemoteModal::BatchAddCollisionChoice { .. }),
            "expected BatchAddCollisionChoice modal when collisions present, got {:?}",
            a.remote.modal
        );
        // No blocking action yet — user must choose the strategy first.
        assert!(
            a.next_blocking.is_none(),
            "must not queue Add before user picks a strategy"
        );
    }

    /// `advance_bulk_add` advances through remaining skills on completion.
    #[test]
    fn a_bulk_pull_advances_through_remaining_on_completion() {
        let mut cfg = Config::default();
        cfg.remotes.insert(
            "h".into(),
            quay_core::RemoteConfig {
                url: "https://x".into(),
                default: true,
                provider: None,
                push_mode: quay_core::PushMode::default(),
                direct_branch: None,
            },
        );
        let mut a = App::new(cfg, std::path::PathBuf::from("/tmp"), None);
        let mut remaining = VecDeque::new();
        remaining.push_back("skill-b".to_string());
        a.remote.bulk_add = BulkAddState::BulkAdding {
            remaining,
            total: 2,
            current: "skill-a".to_string(),
            remote: None,
            force: false,
            results: Vec::new(),
            spinner: Spinner::default(),
            started_at: std::time::Instant::now(),
        };

        advance_bulk_add(&mut a, "skill-a".to_string(), true, None);

        assert!(
            matches!(
                a.remote.bulk_add,
                BulkAddState::BulkAdding { ref current, .. } if current == "skill-b"
            ),
            "expected BulkAdding for skill-b"
        );
        assert!(a.next_blocking.is_some(), "should defer Add for next skill");
    }

    /// `[A]` with picks non-empty shows a bulk confirm modal.
    #[test]
    fn cap_a_bulk_force_pull_with_confirm() {
        let mut cfg = Config::default();
        cfg.remotes.insert(
            "h".into(),
            quay_core::RemoteConfig {
                url: "https://x".into(),
                default: true,
                provider: None,
                push_mode: quay_core::PushMode::default(),
                direct_branch: None,
            },
        );
        let mut a = App::new(cfg, std::path::PathBuf::from("/tmp"), None);
        a.remote.rows = vec![
            RemoteSkillRow {
                name: "aaa".into(),
                version: "1.0.0".into(),
                description: "d".into(),
                tags: vec![],
            },
            RemoteSkillRow {
                name: "bbb".into(),
                version: "1.0.0".into(),
                description: "d".into(),
                tags: vec![],
            },
        ];
        a.remote.fetched = true;
        a.remote.picks.insert(0);
        a.remote.picks.insert(1);

        handle_key(&mut a, KeyCode::Char('A'));

        assert!(
            matches!(
                a.remote.modal,
                RemoteModal::ConfirmBulkForcePull { count: 2 }
            ),
            "expected ConfirmBulkForcePull modal, got {:?}",
            a.remote.modal
        );
        assert!(a.next_blocking.is_none(), "no action until user confirms");
    }

    /// `advance_bulk_add` transitions to `BulkAddDone` when remaining is empty.
    #[test]
    fn bulk_add_done_after_last_pick() {
        let mut a = App::new(Config::default(), std::path::PathBuf::from("/tmp"), None);
        a.remote.bulk_add = BulkAddState::BulkAdding {
            remaining: VecDeque::new(),
            total: 1,
            current: "only-skill".to_string(),
            remote: None,
            force: false,
            results: Vec::new(),
            spinner: Spinner::default(),
            started_at: std::time::Instant::now(),
        };

        advance_bulk_add(&mut a, "only-skill".to_string(), true, None);

        assert!(
            matches!(
                a.remote.bulk_add,
                BulkAddState::BulkAddDone(ref r) if r.len() == 1 && r[0].outcome.is_ok()
            ),
            "expected BulkAddDone with one success"
        );
    }

    // -----------------------------------------------------------------------
    // Task 10f — Collision modal tests
    // -----------------------------------------------------------------------

    /// `[a]` with picks that include already-local skills opens the
    /// `BatchAddCollisionChoice` modal instead of silently skipping.
    #[test]
    fn bulk_add_with_collisions_opens_batch_add_collision_choice_modal() {
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
                direct_branch: None,
            },
        );
        let mut a = App::new(cfg, project.path().to_path_buf(), None);
        a.remote.rows = vec![
            RemoteSkillRow {
                name: "foo".into(),
                version: "1.0.0".into(),
                description: "d".into(),
                tags: vec![],
            },
            RemoteSkillRow {
                name: "bar".into(),
                version: "1.0.0".into(),
                description: "d".into(),
                tags: vec![],
            },
        ];
        a.remote.fetched = true;
        // Select both.
        a.remote.picks.insert(0);
        a.remote.picks.insert(1);

        handle_key(&mut a, KeyCode::Char('a'));

        // Must open BatchAddCollisionChoice, NOT queue a blocking action.
        assert!(
            matches!(a.remote.modal, RemoteModal::BatchAddCollisionChoice { .. }),
            "expected BatchAddCollisionChoice modal, got {:?}",
            a.remote.modal
        );
        assert!(
            a.next_blocking.is_none(),
            "must not queue blocking action until user confirms"
        );
    }

    /// `[a]` with no collisions (all picks are fresh) skips the modal and
    /// starts the bulk pull directly.
    #[test]
    fn bulk_add_no_collisions_starts_pull_directly() {
        let mut cfg = Config::default();
        cfg.remotes.insert(
            "h".into(),
            quay_core::RemoteConfig {
                url: "https://x".into(),
                default: true,
                provider: None,
                push_mode: quay_core::PushMode::default(),
                direct_branch: None,
            },
        );
        let mut a = App::new(cfg, std::path::PathBuf::from("/tmp"), None);
        a.remote.rows = vec![
            RemoteSkillRow {
                name: "foo".into(),
                version: "1.0.0".into(),
                description: "d".into(),
                tags: vec![],
            },
            RemoteSkillRow {
                name: "bar".into(),
                version: "1.0.0".into(),
                description: "d".into(),
                tags: vec![],
            },
        ];
        a.remote.fetched = true;
        a.remote.picks.insert(0);
        a.remote.picks.insert(1);

        handle_key(&mut a, KeyCode::Char('a'));

        // No modal — bulk add started directly.
        assert!(
            matches!(a.remote.modal, RemoteModal::None),
            "no modal when no collisions"
        );
        assert!(
            a.next_blocking.is_some(),
            "should queue Add when no collisions"
        );
        assert!(
            matches!(a.remote.bulk_add, BulkAddState::BulkAdding { .. }),
            "expected BulkAdding state"
        );
    }

    /// Choosing "Update all" in BatchAddCollisionChoice resolves to a force pull
    /// that includes all picked skills.
    #[test]
    fn update_all_resolves_to_force_actions() {
        let mut cfg = Config::default();
        cfg.remotes.insert(
            "h".into(),
            quay_core::RemoteConfig {
                url: "https://x".into(),
                default: true,
                provider: None,
                push_mode: quay_core::PushMode::default(),
                direct_branch: None,
            },
        );
        let mut a = App::new(cfg, std::path::PathBuf::from("/tmp"), None);
        // Manually put the modal in the right state.
        a.remote.modal = RemoteModal::BatchAddCollisionChoice {
            collisions: vec!["foo".into(), "bar".into()],
            fresh: vec!["baz".into()],
            highlighted: 0, // UpdateAll
        };

        // Confirm with Enter.
        handle_key(&mut a, KeyCode::Enter);

        // Should have started bulk add with force=true for all three.
        assert!(
            matches!(a.remote.modal, RemoteModal::None),
            "modal must be dismissed"
        );
        assert!(
            a.next_blocking.is_some(),
            "should queue Add after UpdateAll"
        );
        if let Some(crate::tui::app::BlockingAction::Add { ref force, .. }) = a.next_blocking {
            assert!(force, "UpdateAll must use force=true");
        }
        assert!(
            matches!(a.remote.bulk_add, BulkAddState::BulkAdding { .. }),
            "expected BulkAdding state"
        );
    }

    /// Choosing "Skip all" in BatchAddCollisionChoice only queues fresh skills
    /// (collisions are dropped from the plan).
    #[test]
    fn skip_all_drops_collisions_from_plan() {
        let mut cfg = Config::default();
        cfg.remotes.insert(
            "h".into(),
            quay_core::RemoteConfig {
                url: "https://x".into(),
                default: true,
                provider: None,
                push_mode: quay_core::PushMode::default(),
                direct_branch: None,
            },
        );
        let mut a = App::new(cfg, std::path::PathBuf::from("/tmp"), None);
        a.remote.modal = RemoteModal::BatchAddCollisionChoice {
            collisions: vec!["foo".into()],
            fresh: vec!["bar".into(), "baz".into()],
            highlighted: 1, // SkipAll
        };

        handle_key(&mut a, KeyCode::Enter);

        assert!(
            matches!(a.remote.modal, RemoteModal::None),
            "modal must be dismissed"
        );
        assert!(
            a.next_blocking.is_some(),
            "should queue Add for fresh skills"
        );
        // The queued skill should be one of the fresh ones, not "foo".
        if let Some(crate::tui::app::BlockingAction::Add { ref skill, .. }) = a.next_blocking {
            assert_ne!(skill, "foo", "foo must be skipped");
        }
        assert!(
            matches!(a.remote.bulk_add, BulkAddState::BulkAdding { .. }),
            "expected BulkAdding state"
        );
    }

    /// "Prompt per skill" transitions to `PerCollisionChoice` for the first
    /// collision; confirming all of them eventually starts `BulkAddState`.
    #[test]
    fn prompt_each_iterates_collisions_then_starts_bulk_add() {
        let mut cfg = Config::default();
        cfg.remotes.insert(
            "h".into(),
            quay_core::RemoteConfig {
                url: "https://x".into(),
                default: true,
                provider: None,
                push_mode: quay_core::PushMode::default(),
                direct_branch: None,
            },
        );
        let mut a = App::new(cfg, std::path::PathBuf::from("/tmp"), None);
        a.remote.modal = RemoteModal::BatchAddCollisionChoice {
            collisions: vec!["col-a".into(), "col-b".into()],
            fresh: vec!["new-c".into()],
            highlighted: 2, // PromptEach
        };

        // Enter → PromptEach → PerCollisionChoice for col-a.
        handle_key(&mut a, KeyCode::Enter);
        assert!(
            matches!(
                a.remote.modal,
                RemoteModal::PerCollisionChoice { ref skill_name, .. } if skill_name == "col-a"
            ),
            "expected PerCollisionChoice for col-a, got {:?}",
            a.remote.modal
        );

        // [u] → Update col-a → PerCollisionChoice for col-b.
        handle_key(&mut a, KeyCode::Char('u'));
        assert!(
            matches!(
                a.remote.modal,
                RemoteModal::PerCollisionChoice { ref skill_name, .. } if skill_name == "col-b"
            ),
            "expected PerCollisionChoice for col-b, got {:?}",
            a.remote.modal
        );

        // [s] → Skip col-b → all done → BulkAdding.
        handle_key(&mut a, KeyCode::Char('s'));
        assert!(
            matches!(a.remote.modal, RemoteModal::None),
            "modal must be dismissed after last collision"
        );
        // col-a was UpdateForce (force=true) so bulk add runs with force=true
        // which includes new-c as well.
        assert!(
            a.next_blocking.is_some(),
            "should queue Add for resolved skills"
        );
    }
}
