//! Deciding what happens to local files an update does not contain.
//!
//! `quay-core` computes the extra-file set but cannot prompt — `dialoguer` is a
//! `quay-cli` dependency. This module is the other half: it turns
//! `--keep-extra` / `--delete-extra`, TTY-ness and the user's answer into the
//! `ExtraFiles` verdict core asks for.

use quay_core::{ExtraFiles, QuayError, Result};
use std::cell::Cell;

/// Where the verdict comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtraPolicy {
    /// Prompt when there is a terminal; keep and note when there is not.
    Ask,
    /// `--keep-extra`.
    Keep,
    /// `--delete-extra`.
    Delete,
}

impl ExtraPolicy {
    /// clap's `conflicts_with` rules out both flags at once; the mapping is
    /// still total so a future caller cannot construct a silent surprise. Both
    /// true resolves to `Keep` rather than `Delete` — deleting always takes an
    /// explicit, unambiguous answer, never a fallback.
    pub fn from_flags(keep: bool, delete: bool) -> Self {
        match (keep, delete) {
            (true, _) => ExtraPolicy::Keep,
            (_, true) => ExtraPolicy::Delete,
            _ => ExtraPolicy::Ask,
        }
    }
}

/// Answers core's extra-file question for a whole command invocation.
///
/// One `Decider` spans every skill in an `update`, which is what lets "keep all
/// remaining" and the interrupt flag persist across the loop.
pub struct Decider {
    policy: ExtraPolicy,
    interactive: bool,
    sticky_keep: Cell<bool>,
    interrupted: Cell<bool>,
}

impl Decider {
    pub fn new(policy: ExtraPolicy, interactive: bool) -> Self {
        Self {
            policy,
            interactive,
            sticky_keep: Cell::new(false),
            interrupted: Cell::new(false),
        }
    }

    /// True once a prompt was interrupted. `update`'s interactive loop reports
    /// per-skill errors and keeps going, so without this an interrupt on the
    /// first skill would re-prompt the second and cascade.
    pub fn interrupted(&self) -> bool {
        self.interrupted.get()
    }

    #[cfg(test)]
    fn set_sticky_keep_for_test(&self) {
        self.sticky_keep.set(true);
    }

    /// The `DecideExtras` callback body. Pass as `&|s, e| decider.decide(s, e)`.
    pub fn decide(&self, skill: &str, extras: &[String]) -> Result<ExtraFiles> {
        match self.policy {
            ExtraPolicy::Keep => Ok(ExtraFiles::Keep),
            ExtraPolicy::Delete => Ok(ExtraFiles::Delete),
            ExtraPolicy::Ask => {
                // The sticky answer is itself a human decision, so unlike the
                // plain non-interactive path it does not get a note.
                if !self.interactive {
                    eprintln!("{}", note_text(skill, extras));
                    return Ok(ExtraFiles::Keep);
                }
                if self.sticky_keep.get() {
                    return Ok(ExtraFiles::Keep);
                }
                match self.prompt(skill, extras) {
                    Ok(v) => Ok(v),
                    Err(e) => self.resolve_prompt_error(skill, extras, e),
                }
            }
        }
    }

    fn prompt(
        &self,
        skill: &str,
        extras: &[String],
    ) -> std::result::Result<ExtraFiles, dialoguer::Error> {
        eprintln!("{skill}: {} files not in the new version", extras.len());
        for e in extras {
            eprintln!("  {e}");
        }
        let choice = dialoguer::Select::new()
            .with_prompt("What should happen to them?")
            .items([
                "keep them",
                "delete them",
                "pick which to delete",
                "keep these and all remaining",
            ])
            .default(0)
            .interact()?;
        match choice {
            1 => Ok(ExtraFiles::Delete),
            2 => {
                // Nothing preselected: a stray Enter keeps everything, and
                // deleting takes deliberate keystrokes.
                let picked = dialoguer::MultiSelect::new()
                    .with_prompt("Select the files to DELETE (Space to toggle, Enter to confirm)")
                    .items(extras)
                    .interact()?;
                Ok(ExtraFiles::DeleteOnly(
                    picked.into_iter().map(|i| extras[i].clone()).collect(),
                ))
            }
            3 => {
                self.sticky_keep.set(true);
                Ok(ExtraFiles::Keep)
            }
            _ => Ok(ExtraFiles::Keep),
        }
    }

    /// Turns a failed prompt into either an abort or a degrade.
    ///
    /// `dialoguer` renders on stderr, not stdin — `is_tty()` only checks stdin
    /// — so `quay update 2>somewhere` from a real terminal still takes this
    /// path and must not abort the whole run. Only Ctrl-C
    /// (`ErrorKind::Interrupted`) is a real interrupt; `Select::interact()`
    /// (unlike `interact_opt()`) ignores Esc entirely, so it never produces
    /// one. Every other kind — closed stdin, stderr that turned out not to be
    /// a terminal after all — degrades to the same "keep and note" outcome as
    /// no terminal at all.
    fn resolve_prompt_error(
        &self,
        skill: &str,
        extras: &[String],
        e: dialoguer::Error,
    ) -> Result<ExtraFiles> {
        let dialoguer::Error::IO(io_err) = e;
        if io_err.kind() == std::io::ErrorKind::Interrupted {
            self.interrupted.set(true);
            return Err(QuayError::Io {
                path: "prompt".into(),
                source: io_err,
            });
        }
        eprintln!("{}", note_text(skill, extras));
        Ok(ExtraFiles::Keep)
    }
}

/// The message shown when extras were kept without a human deciding.
fn note_text(skill: &str, extras: &[String]) -> String {
    let n = extras.len();
    let plural = if n == 1 { "file" } else { "files" };
    format!(
        "note: {skill} — kept {n} {plural} not in the new version ({}). \
         Pass --delete-extra to remove them.",
        extras.join(", ")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extras() -> Vec<String> {
        vec!["notes.md".to_string(), "refs/legacy.md".to_string()]
    }

    #[test]
    fn flags_map_to_policies() {
        assert_eq!(ExtraPolicy::from_flags(false, false), ExtraPolicy::Ask);
        assert_eq!(ExtraPolicy::from_flags(true, false), ExtraPolicy::Keep);
        assert_eq!(ExtraPolicy::from_flags(false, true), ExtraPolicy::Delete);
        // clap's conflicts_with makes this unreachable, but the mapping is
        // total, and deleting always takes an explicit answer — never the
        // fallback for an ambiguous combination.
        assert_eq!(ExtraPolicy::from_flags(true, true), ExtraPolicy::Keep);
    }

    #[test]
    fn explicit_flags_never_prompt_even_when_interactive() {
        let keep = Decider::new(ExtraPolicy::Keep, true);
        assert_eq!(
            keep.decide("csv-parse", &extras()).unwrap(),
            ExtraFiles::Keep
        );

        let del = Decider::new(ExtraPolicy::Delete, true);
        assert_eq!(
            del.decide("csv-parse", &extras()).unwrap(),
            ExtraFiles::Delete
        );
    }

    #[test]
    fn ask_without_a_tty_keeps() {
        let d = Decider::new(ExtraPolicy::Ask, false);
        assert_eq!(d.decide("csv-parse", &extras()).unwrap(), ExtraFiles::Keep);
        assert!(!d.interrupted());
    }

    /// The sticky answer is what keeps a twelve-skill update from asking twelve
    /// times. Once set, later skills resolve without touching the terminal —
    /// which is also why this test can run with `interactive: true` in CI.
    #[test]
    fn sticky_keep_suppresses_later_asks() {
        let d = Decider::new(ExtraPolicy::Ask, true);
        d.set_sticky_keep_for_test();
        assert_eq!(d.decide("csv-parse", &extras()).unwrap(), ExtraFiles::Keep);
        assert_eq!(d.decide("other", &extras()).unwrap(), ExtraFiles::Keep);
    }

    #[test]
    fn note_lists_every_extra_file() {
        let msg = note_text("csv-parse", &extras());
        assert!(msg.contains("csv-parse"));
        assert!(msg.contains("notes.md"));
        assert!(msg.contains("refs/legacy.md"));
        assert!(msg.contains("--delete-extra"));
    }

    #[test]
    fn note_text_pluralizes_a_single_file() {
        let msg = note_text("csv-parse", &["notes.md".to_string()]);
        assert!(msg.contains("kept 1 file "), "got: {msg}");
        assert!(!msg.contains("1 files"), "got: {msg}");
    }

    /// Ctrl-C during `dialoguer::Select::interact()` surfaces as
    /// `io::ErrorKind::Interrupted` — the one case that must abort rather than
    /// degrade.
    #[test]
    fn interrupted_error_sets_interrupted_and_propagates() {
        let d = Decider::new(ExtraPolicy::Ask, true);
        let io_err = std::io::Error::new(std::io::ErrorKind::Interrupted, "ctrl-c");
        let result = d.resolve_prompt_error("csv-parse", &extras(), dialoguer::Error::IO(io_err));
        assert!(result.is_err());
        assert!(d.interrupted());
    }

    /// `dialoguer` renders on stderr, not stdin, so a real terminal whose
    /// stderr happens to be redirected surfaces `NotConnected` here — not an
    /// interrupt. It must degrade to `Keep` and leave `interrupted()` false,
    /// the same as no terminal at all.
    #[test]
    fn not_connected_error_degrades_to_keep_without_interrupting() {
        let d = Decider::new(ExtraPolicy::Ask, true);
        let io_err = std::io::Error::new(std::io::ErrorKind::NotConnected, "not a tty");
        let result = d.resolve_prompt_error("csv-parse", &extras(), dialoguer::Error::IO(io_err));
        assert_eq!(result.unwrap(), ExtraFiles::Keep);
        assert!(!d.interrupted());
    }

    /// Closed stdin surfaces as `UnexpectedEof` and must also degrade rather
    /// than abort.
    #[test]
    fn unexpected_eof_error_degrades_to_keep_without_interrupting() {
        let d = Decider::new(ExtraPolicy::Ask, true);
        let io_err = std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "stdin closed");
        let result = d.resolve_prompt_error("csv-parse", &extras(), dialoguer::Error::IO(io_err));
        assert_eq!(result.unwrap(), ExtraFiles::Keep);
        assert!(!d.interrupted());
    }
}
