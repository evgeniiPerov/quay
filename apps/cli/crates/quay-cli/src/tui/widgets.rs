use crate::tui::app::App;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

pub fn render_status_bar(frame: &mut Frame, app: &App, area: Rect) {
    let profile = app
        .cfg
        .user
        .email
        .clone()
        .unwrap_or_else(|| "(no profile)".into());
    let installed = app.lock.skills.len();
    let remotes = app.cfg.remotes.len();
    let body = if let Some(msg) = &app.status_message {
        Line::from(vec![
            Span::raw(" "),
            Span::styled(msg.clone(), Style::default().add_modifier(Modifier::BOLD)),
        ])
    } else {
        Line::from(vec![
            Span::raw(" profile: "),
            Span::styled(profile, Style::default().add_modifier(Modifier::BOLD)),
            Span::raw(format!(" │ {} remotes ", remotes)),
            Span::raw(format!("│ {} installed ", installed)),
            Span::raw("│ [1] dash [2] browse [3] search [4] installed [q] quit"),
        ])
    };
    frame.render_widget(Paragraph::new(body), area);
}
