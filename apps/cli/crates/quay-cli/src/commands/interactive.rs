//! Shared interactive selection helpers for CLI `-i` / `--interactive` mode.
//!
//! Wraps `dialoguer::MultiSelect` and `dialoguer::Select` with a non-TTY
//! fallback that returns a clear error message instead of panicking.

use std::fmt;

/// Error returned when interactive selection is requested but the terminal is
/// not a TTY (e.g. when stdin is piped in CI).
#[derive(Debug)]
pub struct InteractiveUnavailable;

impl fmt::Display for InteractiveUnavailable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "interactive mode (-i) requires a TTY; stdin is not a terminal. \
             Pass skill name(s) directly instead."
        )
    }
}

impl std::error::Error for InteractiveUnavailable {}

/// Present a `dialoguer::MultiSelect` prompt and return the selected indices.
///
/// Returns `Err(InteractiveUnavailable)` when stdin is not a TTY so the caller
/// can propagate a clear error without panicking.
///
/// # Arguments
///
/// * `prompt` – Header text shown above the checkbox list.
/// * `items` – All items to show.
/// * `label` – Closure mapping `&T` to a display `String`.
pub fn pick_many<T, F>(
    prompt: &str,
    items: &[T],
    label: F,
) -> Result<Vec<usize>, Box<dyn std::error::Error>>
where
    F: Fn(&T) -> String,
{
    if !is_tty() {
        return Err(Box::new(InteractiveUnavailable));
    }
    let labels: Vec<String> = items.iter().map(&label).collect();
    let picks = dialoguer::MultiSelect::new()
        .with_prompt(prompt)
        .items(&labels)
        .interact()?;
    Ok(picks)
}

/// Present a `dialoguer::Select` prompt and return the index of the chosen item.
///
/// Returns `Err(InteractiveUnavailable)` when stdin is not a TTY so the caller
/// can propagate a clear error without panicking.
///
/// # Arguments
///
/// * `prompt` – Header text shown above the list.
/// * `items` – All items to show.
/// * `label` – Closure mapping `&T` to a display `String`.
/// * `default` – Index of the item that is highlighted initially (e.g. the
///   currently-active entry). Pass `None` to start at index 0.
pub fn pick_one<T, F>(
    prompt: &str,
    items: &[T],
    label: F,
    default: Option<usize>,
) -> Result<usize, Box<dyn std::error::Error>>
where
    F: Fn(&T) -> String,
{
    if !is_tty() {
        return Err(Box::new(InteractiveUnavailable));
    }
    let labels: Vec<String> = items.iter().map(&label).collect();
    let picked = dialoguer::Select::new()
        .with_prompt(prompt)
        .items(&labels)
        .default(default.unwrap_or(0))
        .interact()?;
    Ok(picked)
}

/// Returns `true` when stdin is a TTY (interactive terminal).
///
/// Uses `std::io::IsTerminal` (stable since Rust 1.70).
pub(crate) fn is_tty() -> bool {
    use std::io::IsTerminal;
    std::io::stdin().is_terminal()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// In the test environment stdin is a pipe (not a TTY), so `pick_many` must
    /// return `InteractiveUnavailable` without attempting to open a prompt.
    #[test]
    fn pick_many_returns_error_when_not_tty() {
        // In CI / cargo test stdin is never a TTY.
        // We cannot force is_tty() to false here, but we can verify the error
        // type on a known non-TTY by checking is_tty() itself.
        // If running interactively this test is vacuously correct.
        if is_tty() {
            // Running in a real terminal — skip the assertion to avoid a blocking
            // prompt. The manual smoke test covers this path.
            return;
        }
        let items = vec!["alpha", "beta"];
        let result = pick_many("Choose", &items, |s| s.to_string());
        assert!(result.is_err(), "expected Err when stdin is not a TTY");
        let err = result.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("TTY") || msg.contains("terminal"),
            "error message should mention TTY: {msg}"
        );
    }

    /// In the test environment stdin is a pipe (not a TTY), so `pick_one` must
    /// return `InteractiveUnavailable` without attempting to open a prompt.
    #[test]
    fn pick_one_returns_error_when_not_tty() {
        if is_tty() {
            // Running in a real terminal — skip the assertion to avoid a blocking
            // prompt. The manual smoke test covers this path.
            return;
        }
        let items = vec!["alpha", "beta"];
        let result = pick_one("Choose one", &items, |s| s.to_string(), Some(0));
        assert!(result.is_err(), "expected Err when stdin is not a TTY");
        let err = result.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("TTY") || msg.contains("terminal"),
            "error message should mention TTY: {msg}"
        );
    }
}
