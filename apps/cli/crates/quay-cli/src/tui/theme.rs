//! Style constants shared across the TUI.

use ratatui::style::{Color, Modifier, Style};

pub fn accent() -> Style {
    Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD)
}

pub fn dim() -> Style {
    Style::default().fg(Color::DarkGray)
}

pub fn selected() -> Style {
    Style::default()
        .bg(Color::Cyan)
        .fg(Color::Black)
        .add_modifier(Modifier::BOLD)
}
