//! Shared dark-palette [`FormStyle`] for all `ratatui-form` forms in the TUI.
//!
//! A single call to [`dark()`] produces a consistently styled form across
//! Onboarding, Settings modals, and the Create/Push frontmatter form.

use ratatui::style::{Color, Modifier, Style};
use ratatui_form::FormStyle;

/// Build the dark-palette [`FormStyle`] used by all forms in the quay TUI.
pub fn dark() -> FormStyle {
    FormStyle::default()
        .title(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .label(Style::default().fg(Color::Gray))
        .label_focused(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .input(Style::default().fg(Color::Gray).bg(Color::DarkGray))
        .input_focused(Style::default().fg(Color::White).bg(Color::DarkGray))
        .placeholder(Style::default().fg(Color::DarkGray))
        .error(Style::default().fg(Color::Red).add_modifier(Modifier::BOLD))
        .button(Style::default().fg(Color::Gray))
        .button_focused(
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
}
