//! Cross-platform "open URL in default browser" helper.
//!
//! Uses `xdg-open` (Linux), `open` (macOS), or `cmd /c start` (Windows).
//! Spawns the helper without waiting — process detaches. Errors only when
//! the helper binary itself can't be launched.

use std::io;
use std::process::Command;

/// Strategy for `open_browser_with` — `Stub` is for tests so they don't actually spawn a browser.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenStrategy {
    System,
    Stub,
}

/// Open `url` in the system default browser.
pub fn open_browser(url: &str) -> io::Result<()> {
    open_browser_with(url, OpenStrategy::System)
}

/// Like [`open_browser`] but lets tests pass `OpenStrategy::Stub` to skip spawning.
pub fn open_browser_with(url: &str, strategy: OpenStrategy) -> io::Result<()> {
    if matches!(strategy, OpenStrategy::Stub) {
        return Ok(());
    }

    #[cfg(target_os = "linux")]
    {
        Command::new("xdg-open").arg(url).spawn()?;
    }
    #[cfg(target_os = "macos")]
    {
        Command::new("open").arg(url).spawn()?;
    }
    #[cfg(target_os = "windows")]
    {
        Command::new("cmd").args(["/c", "start", "", url]).spawn()?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stub_strategy_succeeds_without_spawn() {
        open_browser_with("https://example.com", OpenStrategy::Stub).unwrap();
    }

    #[test]
    fn stub_strategy_works_with_empty_url() {
        // Stub mode never validates the URL — that's the OS opener's job.
        open_browser_with("", OpenStrategy::Stub).unwrap();
    }
}
