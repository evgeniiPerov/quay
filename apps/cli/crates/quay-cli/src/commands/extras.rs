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
    /// still total so a future caller cannot construct a silent surprise.
    pub fn from_flags(keep: bool, delete: bool) -> Self {
        match (keep, delete) {
            (_, true) => ExtraPolicy::Delete,
            (true, _) => ExtraPolicy::Keep,
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
                if !self.interactive || self.sticky_keep.get() {
                    eprintln!("{}", note_text(skill, extras));
                    return Ok(ExtraFiles::Keep);
                }
                self.prompt(skill, extras)
            }
        }
    }

    fn prompt(&self, skill: &str, extras: &[String]) -> Result<ExtraFiles> {
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
            .interact()
            .map_err(|e| self.prompt_err(e))?;
        match choice {
            1 => Ok(ExtraFiles::Delete),
            2 => {
                // Nothing preselected: a stray Enter keeps everything, and
                // deleting takes deliberate keystrokes.
                let picked = dialoguer::MultiSelect::new()
                    .with_prompt("Select the files to DELETE (Space to toggle, Enter to confirm)")
                    .items(extras)
                    .interact()
                    .map_err(|e| self.prompt_err(e))?;
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

    /// `dialoguer::Error` wraps an `io::Error`, so this needs no new
    /// `QuayError` variant. Recording the interrupt here rather than inspecting
    /// the kind later keeps the one caller that loops from having to classify
    /// errors it did not raise.
    fn prompt_err(&self, e: dialoguer::Error) -> QuayError {
        self.interrupted.set(true);
        QuayError::Io {
            path: "prompt".into(),
            source: std::io::Error::other(e.to_string()),
        }
    }
}

/// The message shown when extras were kept without a human deciding.
fn note_text(skill: &str, extras: &[String]) -> String {
    format!(
        "note: {skill} — kept {} files not in the new version ({}). \
         Pass --delete-extra to remove them.",
        extras.len(),
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
        // clap's conflicts_with makes this unreachable, but the mapping is total.
        assert_eq!(ExtraPolicy::from_flags(true, true), ExtraPolicy::Delete);
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
}
