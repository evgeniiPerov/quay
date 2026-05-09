//! Synchronous `$EDITOR` suspend/resume helper.
//!
//! The TUI must leave raw/alternate-screen mode before spawning an external
//! editor, then restore the terminal when the editor exits.  This module
//! encapsulates that sequence so any screen can reuse it without duplicating
//! the crossterm dance.

use crossterm::{
    event::{DisableBracketedPaste, EnableBracketedPaste},
    execute,
    terminal::{
        disable_raw_mode, enable_raw_mode, Clear, ClearType, EnterAlternateScreen,
        LeaveAlternateScreen,
    },
};
use std::{io::stdout, path::Path, process::Command};

/// Suspend the TUI, open `path` in `$EDITOR` (falling back to `vi`), then
/// restore the terminal.
///
/// Returns `Ok(())` if the editor exits with a zero status.  Returns
/// `Err(...)` for I/O failures or a non-zero editor exit status.
pub fn run_editor(path: &Path) -> std::io::Result<()> {
    // Disable bracketed paste before suspending — the external editor manages
    // its own terminal state.
    let _ = execute!(stdout(), DisableBracketedPaste);
    disable_raw_mode()?;
    execute!(stdout(), LeaveAlternateScreen)?;

    let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vi".into());
    let status = Command::new(&editor).arg(path).status()?;

    enable_raw_mode()?;
    execute!(stdout(), EnterAlternateScreen, Clear(ClearType::All))?;
    // Re-enable bracketed paste now that the TUI is back in control.
    let _ = execute!(stdout(), EnableBracketedPaste);

    if !status.success() {
        return Err(std::io::Error::other(format!(
            "editor `{}` exited non-zero",
            editor
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // `#[ignore]` because `std::env::set_var` is not safe to call concurrently
    // from multiple test threads (UB under POSIX).  The test mutates the
    // `EDITOR` environment variable and must run in a single-threaded context.
    // Run manually with:
    //   cargo test -p quay-cli tui::editor::tests::succeeds_with_true_editor -- --ignored
    #[test]
    #[ignore]
    fn succeeds_with_true_editor() {
        let f = assert_fs::NamedTempFile::new("body.md").unwrap();
        std::fs::write(f.path(), "x").unwrap();
        // SAFETY: this is the only thread mutating EDITOR in this test binary
        // when run with `-- --ignored` (single-threaded runner).
        unsafe {
            std::env::set_var("EDITOR", "true");
        }
        assert!(run_editor(f.path()).is_ok());
    }
}
