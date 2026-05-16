//! Modal shown when a TUI pull hits a collision. State + key transitions only;
//! rendering follows the crate's existing modal render pattern.

use quay_core::reconcile::action::ResolveAction;
use quay_core::reconcile::ReconcileReport;

/// State for the reconcile collision modal.
#[derive(Debug, Clone)]
pub struct ReconcileModal {
    /// Name of the skill being resolved.
    pub skill: String,
    /// The reconcile report produced by the background worker.
    pub report: ReconcileReport,
    /// Scroll offset for the diff body.
    pub scroll: u16,
}

/// What the modal produces after a key press.
#[derive(Debug, PartialEq, Eq)]
pub enum ModalOutcome {
    /// The user chose an action; apply it to this skill.
    Resolved(ResolveAction),
    /// The user dismissed the modal with `Esc`/`q` — no action.
    Dismissed,
    /// The key was consumed but produced no terminal outcome; re-render.
    Continue,
}

impl ReconcileModal {
    /// Create a new modal for `skill` with the given reconcile report.
    pub fn new(skill: String, report: ReconcileReport) -> Self {
        Self {
            skill,
            report,
            scroll: 0,
        }
    }

    /// Map a key character to a [`ModalOutcome`].
    ///
    /// All actions are **single-skill** (apply to the current skill only):
    /// - `r` / `R` — Replace (overwrite local with harbor HEAD). Disabled when
    ///   `report.absent_on_head` is true (nothing to replace with).
    /// - `k` / `K` — Keep (leave local as-is, deliberate choice).
    /// - `s` / `S` — Skip (leave local, undecided).
    /// - `j` — scroll diff down.
    /// - `u` — scroll diff up.
    /// - `q` — dismiss.
    /// - anything else — `Continue` (no-op).
    ///
    /// Note: uppercase `R`/`K`/`S` do NOT apply to all remaining skills —
    /// they behave identically to their lowercase counterparts (single-skill).
    // TODO(deferred): uppercase R/K/S "apply to all remaining" (bulk) not yet
    // implemented — see spec 2026-05-15 deferred items.
    pub fn on_key(&mut self, c: char) -> ModalOutcome {
        match c {
            'r' | 'R' if !self.report.absent_on_head => {
                ModalOutcome::Resolved(ResolveAction::Replace)
            }
            'k' | 'K' => ModalOutcome::Resolved(ResolveAction::Keep),
            's' | 'S' => ModalOutcome::Resolved(ResolveAction::Skip),
            'j' => {
                self.scroll = self.scroll.saturating_add(1);
                ModalOutcome::Continue
            }
            'u' => {
                self.scroll = self.scroll.saturating_sub(1);
                ModalOutcome::Continue
            }
            'q' => self.dismiss(),
            _ => ModalOutcome::Continue,
        }
    }

    fn dismiss(&self) -> ModalOutcome {
        ModalOutcome::Dismissed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quay_core::reconcile::verdict::Verdict;

    #[test]
    fn replace_key_resolves_replace() {
        let report = quay_core::reconcile::report_for_test(
            Verdict::ChangedUnknownDirection { local_edited: true },
            false,
        );
        let mut m = ReconcileModal::new("foo".into(), report);
        assert_eq!(
            m.on_key('r'),
            ModalOutcome::Resolved(ResolveAction::Replace)
        );
    }

    #[test]
    fn replace_disabled_when_absent_on_head() {
        let report = quay_core::reconcile::report_for_test(
            Verdict::ChangedUnknownDirection { local_edited: true },
            true,
        );
        let mut m = ReconcileModal::new("foo".into(), report);
        assert_eq!(m.on_key('r'), ModalOutcome::Continue);
        assert_eq!(m.on_key('k'), ModalOutcome::Resolved(ResolveAction::Keep));
    }

    #[test]
    fn scroll_up_decrements_saturating() {
        let report = quay_core::reconcile::report_for_test(
            Verdict::ChangedUnknownDirection { local_edited: true },
            false,
        );
        let mut m = ReconcileModal::new("foo".into(), report);

        // scroll==0: u is a no-op (saturating), returns Continue.
        assert_eq!(m.on_key('u'), ModalOutcome::Continue);
        assert_eq!(m.scroll, 0, "scroll must stay 0 at floor");

        // j increments scroll.
        assert_eq!(m.on_key('j'), ModalOutcome::Continue);
        assert_eq!(m.scroll, 1);

        // u decrements back to 0.
        assert_eq!(m.on_key('u'), ModalOutcome::Continue);
        assert_eq!(m.scroll, 0);
    }
}
