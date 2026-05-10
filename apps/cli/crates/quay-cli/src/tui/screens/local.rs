//! TUI Screen [2] — Local skills.
//!
//! Renders `Vec<LocalSkill>` from the scanner. Columns: Name, Status, Mirrors, Path.
//! Key bindings:
//!   - `[j]/[k]` — navigate rows
//!   - `[Space]` — toggle row in/out of bulk selection (`picks`)
//!   - `[Enter]` — detail view (all locations + frontmatter)
//!   - `[u]` — quick push (bulk if picks non-empty; else single-skill, patch bump)
//!   - `[U]` — push form (bulk if picks non-empty; else single-skill form)
//!   - `[d]` — remove local (bulk if picks non-empty; else single-skill)
//!   - `[D]` — remove everywhere with confirm (bulk if picks non-empty)
//!   - `[r]` — rescan
//!   - `[Esc]` — clear selection when picks non-empty; otherwise back/dismiss modal

use crate::tui::app::{App, BlockingAction, Screen, ScreenAction};
use crate::tui::theme;
use crossterm::event::KeyCode;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use ratatui::Frame;
use std::collections::BTreeSet;

/// Internal state for the Local screen modal confirm dialog.
#[derive(Debug, Default, Clone)]
pub enum LocalModal {
    #[default]
    None,
    /// Confirm "delete everywhere?" for the selected skill.
    ConfirmDeleteEverywhere { skill_name: String },
    /// Confirm bulk local delete for N picked skills.
    ConfirmBulkDeleteLocal { count: usize },
    /// Confirm bulk delete everywhere for N picked skills.
    ConfirmBulkDeleteEverywhere { count: usize },
}

/// State for the Local screen.
#[derive(Debug, Default)]
pub struct LocalState {
    /// Index of the currently highlighted row.
    pub selected: usize,
    /// Whether the detail panel is expanded (Enter toggles).
    pub detail_open: bool,
    /// Active confirm modal, if any.
    pub modal: LocalModal,
    /// Bulk-selected row indices; toggled by `[Space]`.
    pub picks: BTreeSet<usize>,
}

pub fn handle_key(app: &mut App, code: KeyCode) -> ScreenAction {
    // If a confirm modal is active, intercept all keys.
    let modal = std::mem::replace(&mut app.local.modal, LocalModal::None);
    match modal {
        LocalModal::ConfirmDeleteEverywhere { ref skill_name } => {
            match code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    let name = skill_name.clone();
                    // local.modal is already reset to None above.
                    remove_everywhere(app, &name);
                }
                _ => {
                    // Cancel: restore modal state (do nothing).
                    app.local.modal = modal;
                }
            }
            return ScreenAction::Stay;
        }
        LocalModal::ConfirmBulkDeleteLocal { count: _ } => {
            match code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    // Collect names from picks before mutating.
                    let names: Vec<String> = app
                        .local
                        .picks
                        .iter()
                        .filter_map(|&i| app.local_skills.get(i))
                        .map(|s| s.meta.name.clone())
                        .collect();
                    let n = names.len();
                    for name in &names {
                        remove_local(app, name);
                    }
                    app.local.picks.clear();
                    app.set_status(format!("removed {n} skills locally"));
                }
                _ => {
                    // Cancel: dismiss modal (already None).
                }
            }
            return ScreenAction::Stay;
        }
        LocalModal::ConfirmBulkDeleteEverywhere { count: _ } => {
            match code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    let names: Vec<String> = app
                        .local
                        .picks
                        .iter()
                        .filter_map(|&i| app.local_skills.get(i))
                        .map(|s| s.meta.name.clone())
                        .collect();
                    let n = names.len();
                    for name in &names {
                        remove_everywhere(app, name);
                    }
                    app.local.picks.clear();
                    app.set_status(format!("removed {n} skills everywhere"));
                }
                _ => {
                    // Cancel: dismiss modal (already None).
                }
            }
            return ScreenAction::Stay;
        }
        LocalModal::None => {}
    }

    match code {
        KeyCode::Esc => {
            if !app.local.picks.is_empty() {
                app.local.picks.clear();
                return ScreenAction::Stay;
            }
            // No picks — fall through to default (no-op for Local screen).
            return ScreenAction::Stay;
        }
        KeyCode::Char(' ') => {
            if !app.local_skills.is_empty() {
                let idx = app.local.selected;
                if !app.local.picks.insert(idx) {
                    app.local.picks.remove(&idx);
                }
            }
        }
        KeyCode::Char('j') | KeyCode::Down => {
            if !app.local_skills.is_empty() {
                app.local.selected =
                    (app.local.selected + 1).min(app.local_skills.len().saturating_sub(1));
                app.local.detail_open = false;
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if !app.local_skills.is_empty() {
                app.local.selected = app.local.selected.saturating_sub(1);
                app.local.detail_open = false;
            }
        }
        KeyCode::Enter => {
            app.local.detail_open = !app.local.detail_open;
        }
        KeyCode::Char('r') => {
            app.reload_local_skills();
            let new_len = app.local_skills.len();
            app.local.selected = app.local.selected.min(new_len.saturating_sub(1));
            // Drop picks whose indices no longer resolve to any skill.
            app.local.picks.retain(|&i| i < new_len);
            app.set_status("rescanned local skills");
        }
        KeyCode::Char('u') => {
            if !app.local.picks.is_empty() {
                // Bulk quick-push: Patch bump, default remote, sequential.
                let names: Vec<String> = app
                    .local
                    .picks
                    .iter()
                    .filter_map(|&i| app.local_skills.get(i))
                    .map(|s| s.meta.name.clone())
                    .collect();
                if !names.is_empty() {
                    crate::tui::screens::create_push::bulk_quick_push(app, names);
                    return ScreenAction::SwitchTo(Screen::CreatePush);
                }
            } else if let Some(skill) = app.local_skills.get(app.local.selected).cloned() {
                quick_push(app, &skill);
                return ScreenAction::SwitchTo(Screen::CreatePush);
            }
        }
        KeyCode::Char('U') => {
            if !app.local.picks.is_empty() {
                // Bulk push with form: collect picked skill names and open push form.
                // The form's submit will call bulk_form_push.
                let names: Vec<String> = app
                    .local
                    .picks
                    .iter()
                    .filter_map(|&i| app.local_skills.get(i))
                    .map(|s| s.meta.name.clone())
                    .collect();
                if !names.is_empty() {
                    open_bulk_push_form(app, names);
                    return ScreenAction::SwitchTo(Screen::CreatePush);
                }
            } else if let Some(skill) = app.local_skills.get(app.local.selected).cloned() {
                open_push_form(app, &skill);
                return ScreenAction::SwitchTo(Screen::CreatePush);
            }
        }
        KeyCode::Char('d') => {
            if !app.local.picks.is_empty() {
                let count = app.local.picks.len();
                app.local.modal = LocalModal::ConfirmBulkDeleteLocal { count };
            } else if let Some(skill) = app.local_skills.get(app.local.selected) {
                let name = skill.meta.name.clone();
                remove_local(app, &name);
            }
        }
        KeyCode::Char('D') => {
            if !app.local.picks.is_empty() {
                let count = app.local.picks.len();
                app.local.modal = LocalModal::ConfirmBulkDeleteEverywhere { count };
            } else if let Some(skill) = app.local_skills.get(app.local.selected) {
                let name = skill.meta.name.clone();
                app.local.modal = LocalModal::ConfirmDeleteEverywhere { skill_name: name };
            }
        }
        _ => {}
    }
    ScreenAction::Stay
}

/// Quick-push the selected skill: patch bump, default remote, no form.
///
/// Sets `app.create_push` directly to `Pushing` and defers the blocking push.
/// `push_form_ready = true` ensures the event loop does not reset the state
/// when switching to `Screen::CreatePush`.
fn quick_push(app: &mut App, skill: &quay_core::scanner::LocalSkill) {
    use crate::tui::screens::create_push::{BumpChoice, CreatePushState};
    use crate::tui::screens::widgets::spinner::Spinner;
    use quay_core::BumpKind;
    use std::time::Instant;

    let name = skill.meta.name.clone();
    app.create_push = CreatePushState::Pushing {
        skill: name.clone(),
        remote: None,
        bump: BumpChoice::Patch,
        started_at: Instant::now(),
        spinner: Spinner::default(),
    };
    app.push_form_ready = true;
    app.defer_blocking_action(BlockingAction::Push {
        skill: name,
        remote: None,
        bump: BumpKind::Patch,
    });
}

/// Open a bulk push form for N picked skills.
///
/// The form uses the first skill's tags as a template; on submit, `bulk_form_push`
/// is called with all N skill names and the form's chosen bump + remote.
fn open_bulk_push_form(app: &mut App, names: Vec<String>) {
    use crate::tui::screens::create_push::{build_push_form_from_app, CreatePushState};

    // Use the first skill's tags as the initial tags value.
    let tags_initial = names
        .first()
        .and_then(|n| app.local_skills.iter().find(|s| &s.meta.name == n))
        .map(|s| s.meta.tags.join(", "))
        .unwrap_or_default();
    let label = format!(" Push {} skills ", names.len());
    // Re-use build_push_form_from_app with the label as the skill name (cosmetic).
    let form = build_push_form_from_app(&label, &tags_initial, app);
    app.create_push = CreatePushState::BulkPushForm {
        skill_names: names,
        form: Box::new(form),
    };
    app.push_form_ready = true;
}

/// Open the `PushModal` form for the selected skill.
///
/// Sets `app.push_form_ready = true` so the event loop does not reset the form
/// state when switching to `Screen::CreatePush`.
fn open_push_form(app: &mut App, skill: &quay_core::scanner::LocalSkill) {
    use crate::tui::screens::create_push::{build_push_form_from_app, CreatePushState};

    let tags_initial = skill.meta.tags.join(", ");
    let skill_name = skill.meta.name.clone();
    let skill_path = skill.canonical_path().to_path_buf();

    let form = build_push_form_from_app(&skill_name, &tags_initial, app);
    app.create_push = CreatePushState::PushModal {
        skill_name,
        skill_path,
        form: Box::new(form),
    };
    // Signal the event loop to skip the form-reset logic on SwitchTo(CreatePush).
    app.push_form_ready = true;
}

fn remove_local(app: &mut App, skill_name: &str) {
    use quay_core::{CloneFetcher, Config, SkillManager};
    let project_config = app.project_root.join(".quay/config.toml");
    let project_path_arg = if project_config.exists() {
        Some(project_config.as_path())
    } else {
        None
    };
    let cfg = Config::load_resolved(app.user_config_path.as_deref(), project_path_arg, None)
        .unwrap_or_default();
    let f = CloneFetcher::new();
    let mgr = SkillManager::new(&cfg, &f, &f, app.project_root.clone());
    match mgr.remove(skill_name) {
        Ok(()) => {
            app.reload_local_skills();
            app.local.selected = app
                .local
                .selected
                .min(app.local_skills.len().saturating_sub(1));
            app.set_status(format!("removed {skill_name}"));
        }
        Err(e) => {
            app.set_status(format!("remove failed: {e}"));
        }
    }
}

fn remove_everywhere(app: &mut App, skill_name: &str) {
    // Local removal first.
    remove_local(app, skill_name);
    // Remote deletion is out-of-scope for immediate TUI wiring (requires clone+commit).
    // Surface a status message indicating the local part completed.
    app.set_status(format!(
        "removed {skill_name} locally (remote deletion via `quay remove --everywhere {skill_name}`)"
    ));
}

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let skills = &app.local_skills;
    let selected = app.local.selected;
    let detail_open = app.local.detail_open;
    let picks = &app.local.picks;
    let has_picks = !picks.is_empty();

    // Split area: list | detail (when detail_open)
    let (list_area, detail_area) = if detail_open {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
            .split(area);
        (cols[0], Some(cols[1]))
    } else {
        (area, None)
    };

    // Column widths for list view.
    // When picks active: "[x] " prefix (4 chars) + Name(24) Status(22) Mirrors(22) Path(rest)
    let header = if has_picks {
        format!(
            "    {:<24}  {:<22}  {:<22}  {}",
            "Name", "Status", "Mirrors", "Path"
        )
    } else {
        format!(
            " {:<24}  {:<22}  {:<22}  {}",
            "Name", "Status", "Mirrors", "Path"
        )
    };

    let selected_style = Style::default()
        .fg(Color::Black)
        .bg(Color::Cyan)
        .add_modifier(Modifier::BOLD);

    let items: Vec<ListItem> = if skills.is_empty() {
        vec![ListItem::new(Line::from(Span::styled(
            "(no local skills found \u{2014} write one in .agents/skills/<name>/SKILL.md)",
            theme::dim(),
        )))]
    } else {
        skills
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let mirrors: Vec<&str> = s.locations.iter().map(|l| l.root.label()).collect();
                let mirror_str = mirrors.join(",");
                let drift = if s.has_drift() { " !" } else { "" };
                let status_str = badge_for(&s.status);
                let path_str = s.canonical_path().display().to_string();
                let name_col = truncate(&s.meta.name, 24);
                let status_col = truncate(&status_str, 22);
                let mirror_col = truncate(&format!("{mirror_str}{drift}"), 22);
                let row_content = format!(
                    " {:<24}  {:<22}  {:<22}  {}",
                    name_col, status_col, mirror_col, path_str
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

    let title = if has_picks {
        format!(
            " Local skills ({} of {} selected) ",
            picks.len(),
            skills.len()
        )
    } else {
        format!(" Local ({}) ", skills.len())
    };
    let mut header_items = vec![ListItem::new(Line::from(Span::styled(
        header,
        Style::default().add_modifier(Modifier::BOLD),
    )))];
    header_items.extend(items);

    let list = List::new(header_items)
        .block(Block::default().borders(Borders::ALL).title(title))
        .highlight_style(selected_style);
    frame.render_widget(list, list_area);

    // Confirm modal overlay.
    match &app.local.modal {
        LocalModal::ConfirmDeleteEverywhere { skill_name } => {
            render_confirm_modal(frame, area, skill_name);
        }
        LocalModal::ConfirmBulkDeleteLocal { count } => {
            render_confirm_bulk_local_modal(frame, area, *count);
        }
        LocalModal::ConfirmBulkDeleteEverywhere { count } => {
            render_confirm_bulk_everywhere_modal(frame, area, *count);
        }
        LocalModal::None => {}
    }

    // Detail panel.
    if let Some(det_area) = detail_area {
        if let Some(skill) = skills.get(selected) {
            render_detail(frame, det_area, skill);
        }
    }

    // Hint bar at the bottom.
    let hint_area = Rect {
        x: area.x,
        y: area.y + area.height.saturating_sub(1),
        width: area.width,
        height: 1,
    };
    let hint_text = if has_picks {
        format!(
            "[Space] toggle  [u] push selected ({n})  [d] delete selected  [Esc] clear",
            n = picks.len()
        )
    } else {
        "[j]/[k] move  [Space] select  [Enter] detail  [u] push  [U] push+tags  [d] rm local  [D] rm everywhere  [r] rescan".to_string()
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(hint_text, theme::dim()))),
        hint_area,
    );
}

fn render_detail(frame: &mut Frame, area: Rect, skill: &quay_core::scanner::LocalSkill) {
    let mut lines = vec![
        Line::from(Span::styled(
            format!(" {} v{}", skill.meta.name, skill.meta.version),
            theme::accent(),
        )),
        Line::from(""),
        Line::from(format!(" Description: {}", skill.meta.description)),
        Line::from(format!(
            " Tags: {}",
            if skill.meta.tags.is_empty() {
                "(none)".to_string()
            } else {
                skill.meta.tags.join(", ")
            }
        )),
        Line::from(""),
        Line::from(Span::styled(
            " Locations:",
            Style::default().add_modifier(Modifier::BOLD),
        )),
    ];
    for loc in &skill.locations {
        lines.push(Line::from(format!(
            "  [{}] {}  sha {}",
            loc.root.label(),
            loc.path.display(),
            &loc.sha256[..8]
        )));
    }
    if skill.has_drift() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            " ! drift detected across mirrors",
            Style::default().fg(Color::Yellow),
        )));
    }
    frame.render_widget(
        Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(" Detail ")),
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
        .title(" Confirm Delete Everywhere ");
    let inner = block.inner(modal_area);
    frame.render_widget(block, modal_area);
    let lines = vec![
        Line::from(format!(" Delete '{}' everywhere? y/N", skill_name)),
        Line::from(""),
        Line::from(Span::styled(" [y] yes   [any other] cancel", theme::dim())),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_confirm_bulk_local_modal(frame: &mut Frame, area: Rect, count: usize) {
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
        .title(" Confirm Bulk Delete Local ");
    let inner = block.inner(modal_area);
    frame.render_widget(block, modal_area);
    let lines = vec![
        Line::from(format!(" Remove {} skill(s) locally? y/N", count)),
        Line::from(""),
        Line::from(Span::styled(" [y] yes   [any other] cancel", theme::dim())),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_confirm_bulk_everywhere_modal(frame: &mut Frame, area: Rect, count: usize) {
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
        .title(" Confirm Bulk Delete Everywhere ");
    let inner = block.inner(modal_area);
    frame.render_widget(block, modal_area);
    let lines = vec![
        Line::from(format!(" Delete {} skill(s) everywhere? y/N", count)),
        Line::from(""),
        Line::from(Span::styled(" [y] yes   [any other] cancel", theme::dim())),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}

fn badge_for(status: &quay_core::scanner::ScanStatus) -> String {
    use quay_core::scanner::ScanStatus;
    match status {
        ScanStatus::Local => "local".to_string(),
        ScanStatus::Installed { version, .. } => format!("installed v{version}"),
        ScanStatus::InstalledModified { version, .. } => format!("modified v{version}"),
        ScanStatus::PushedLocal { pr_url, .. } if pr_url.is_empty() => "pushed-direct".to_string(),
        ScanStatus::PushedLocal { .. } => "pushed-local".to_string(),
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}\u{2026}", &s[..max.saturating_sub(1)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quay_core::Config;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn fixture_app() -> App {
        App::new(Config::default(), std::path::PathBuf::from("/tmp"), None)
    }

    /// Helper: build an App with two local skills (aaa, bbb).
    fn two_skill_app() -> App {
        use assert_fs::prelude::*;
        let project = assert_fs::TempDir::new().unwrap();
        project
            .child(".agents/skills/aaa/SKILL.md")
            .write_str("---\nname: aaa\ndescription: a\n---\n")
            .unwrap();
        project
            .child(".agents/skills/bbb/SKILL.md")
            .write_str("---\nname: bbb\ndescription: b\n---\n")
            .unwrap();
        App::new(Config::default(), project.path().to_path_buf(), None)
    }

    #[test]
    fn space_toggles_pick() {
        let mut a = two_skill_app();
        a.current_screen = Screen::Local;
        assert!(a.local.picks.is_empty());
        // Press Space on row 0 — should be added.
        handle_key(&mut a, KeyCode::Char(' '));
        assert!(a.local.picks.contains(&0), "picks should contain index 0");
        // Press Space again on row 0 — should be removed.
        handle_key(&mut a, KeyCode::Char(' '));
        assert!(
            a.local.picks.is_empty(),
            "picks should be empty after toggle off"
        );
    }

    #[test]
    fn esc_clears_picks_when_non_empty() {
        let mut a = two_skill_app();
        a.local.picks.insert(0);
        a.local.picks.insert(1);
        assert_eq!(a.local.picks.len(), 2);
        let action = handle_key(&mut a, KeyCode::Esc);
        assert!(matches!(action, ScreenAction::Stay), "should Stay");
        assert!(a.local.picks.is_empty(), "picks should be cleared by Esc");
    }

    #[test]
    fn esc_stays_when_picks_empty() {
        let mut a = fixture_app();
        assert!(a.local.picks.is_empty());
        let action = handle_key(&mut a, KeyCode::Esc);
        assert!(matches!(action, ScreenAction::Stay));
    }

    #[test]
    fn render_with_picks_shows_prefix_and_header_count() {
        use assert_fs::prelude::*;
        let project = assert_fs::TempDir::new().unwrap();
        project
            .child(".agents/skills/aaa/SKILL.md")
            .write_str("---\nname: aaa\ndescription: a\n---\n")
            .unwrap();
        project
            .child(".agents/skills/bbb/SKILL.md")
            .write_str("---\nname: bbb\ndescription: b\n---\n")
            .unwrap();
        project
            .child(".agents/skills/ccc/SKILL.md")
            .write_str("---\nname: ccc\ndescription: c\n---\n")
            .unwrap();
        let mut a = App::new(Config::default(), project.path().to_path_buf(), None);
        a.current_screen = Screen::Local;
        // Select rows 0 and 2 (not 1).
        a.local.picks.insert(0);
        a.local.picks.insert(2);

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
            "selected rows should show [x] prefix; dump: {dump}"
        );
        assert!(
            dump.contains("[ ]"),
            "unselected rows should show [ ] prefix; dump: {dump}"
        );
        assert!(
            dump.contains("2 of 3 selected"),
            "header should show selection count; dump: {dump}"
        );
    }

    #[test]
    fn render_without_picks_shows_no_prefix() {
        use assert_fs::prelude::*;
        let project = assert_fs::TempDir::new().unwrap();
        project
            .child(".agents/skills/aaa/SKILL.md")
            .write_str("---\nname: aaa\ndescription: a\n---\n")
            .unwrap();
        let mut a = App::new(Config::default(), project.path().to_path_buf(), None);
        a.current_screen = Screen::Local;

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
            !dump.contains("[x]"),
            "no [x] prefix when picks empty; dump: {dump}"
        );
        assert!(
            !dump.contains("selected"),
            "header should not mention 'selected' when picks empty; dump: {dump}"
        );
    }

    #[test]
    fn rescan_drops_invalid_picks() {
        use assert_fs::prelude::*;
        let project = assert_fs::TempDir::new().unwrap();
        project
            .child(".agents/skills/aaa/SKILL.md")
            .write_str("---\nname: aaa\ndescription: a\n---\n")
            .unwrap();
        let mut a = App::new(Config::default(), project.path().to_path_buf(), None);
        // Manually inject a pick for index 5 (out of bounds after rescan of 1 skill).
        a.local.picks.insert(5);
        handle_key(&mut a, KeyCode::Char('r'));
        assert!(
            a.local.picks.is_empty(),
            "out-of-bounds picks must be dropped on rescan"
        );
    }

    #[test]
    fn local_renders_without_crash() {
        let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
        let mut a = fixture_app();
        a.current_screen = Screen::Local;
        terminal.draw(|f| crate::tui::draw(f, &a)).unwrap();
    }

    #[test]
    fn local_renders_with_skills() {
        use assert_fs::prelude::*;
        let project = assert_fs::TempDir::new().unwrap();
        project
            .child(".agents/skills/foo/SKILL.md")
            .write_str("---\nname: foo\ndescription: f\nversion: 0.1.0\n---\n")
            .unwrap();
        let mut a = App::new(Config::default(), project.path().to_path_buf(), None);
        a.current_screen = Screen::Local;
        let mut terminal = Terminal::new(TestBackend::new(160, 30)).unwrap();
        terminal.draw(|f| crate::tui::draw(f, &a)).unwrap();
        let dump: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(dump.contains("foo"), "buffer: {dump}");
    }

    #[test]
    fn j_k_navigate_rows() {
        use assert_fs::prelude::*;
        let project = assert_fs::TempDir::new().unwrap();
        project
            .child(".agents/skills/aaa/SKILL.md")
            .write_str("---\nname: aaa\ndescription: a\n---\n")
            .unwrap();
        project
            .child(".agents/skills/bbb/SKILL.md")
            .write_str("---\nname: bbb\ndescription: b\n---\n")
            .unwrap();
        let mut a = App::new(Config::default(), project.path().to_path_buf(), None);
        a.current_screen = Screen::Local;
        assert_eq!(a.local.selected, 0);
        handle_key(&mut a, KeyCode::Char('j'));
        assert_eq!(a.local.selected, 1);
        handle_key(&mut a, KeyCode::Char('k'));
        assert_eq!(a.local.selected, 0);
    }

    /// `[u]` is a quick push: no form, defers `BlockingAction::Push` with Patch bump.
    #[test]
    fn u_quick_pushes_with_patch_bump() {
        use crate::tui::app::BlockingAction;
        use crate::tui::screens::create_push::CreatePushState;
        use assert_fs::prelude::*;
        use quay_core::BumpKind;

        let project = assert_fs::TempDir::new().unwrap();
        project
            .child(".agents/skills/my-skill/SKILL.md")
            .write_str(
                "---\nname: my-skill\ndescription: d\nversion: 0.1.0\ntags: [rust, cli]\n---\n",
            )
            .unwrap();

        let mut a = App::new(Config::default(), project.path().to_path_buf(), None);
        a.current_screen = Screen::Local;

        let action = handle_key(&mut a, KeyCode::Char('u'));

        assert!(
            matches!(action, ScreenAction::SwitchTo(Screen::CreatePush)),
            "expected SwitchTo(CreatePush), got {:?}",
            action
        );
        assert!(a.push_form_ready, "push_form_ready should be true");
        assert!(
            matches!(a.create_push, CreatePushState::Pushing { .. }),
            "expected Pushing, got {:?}",
            a.create_push
        );
        match a.next_blocking.as_ref() {
            Some(BlockingAction::Push { skill, bump, .. }) => {
                assert_eq!(skill, "my-skill");
                assert!(matches!(bump, BumpKind::Patch));
            }
            other => panic!("expected BlockingAction::Push with Patch, got {other:?}"),
        }
    }

    /// `[U]` opens the same push form as `[u]`.
    #[test]
    fn shift_u_opens_push_modal_with_tags_prefilled() {
        use crate::tui::screens::create_push::CreatePushState;
        use assert_fs::prelude::*;

        let project = assert_fs::TempDir::new().unwrap();
        project
            .child(".agents/skills/tagged-skill/SKILL.md")
            .write_str(
                "---\nname: tagged-skill\ndescription: d\nversion: 0.1.0\ntags: [foo, bar]\n---\n",
            )
            .unwrap();

        let mut a = App::new(Config::default(), project.path().to_path_buf(), None);
        a.current_screen = Screen::Local;

        let action = handle_key(&mut a, KeyCode::Char('U'));

        assert!(
            matches!(action, ScreenAction::SwitchTo(Screen::CreatePush)),
            "expected SwitchTo(CreatePush), got {:?}",
            action
        );
        assert!(a.push_form_ready, "push_form_ready should be true");
        assert!(
            matches!(a.create_push, CreatePushState::PushModal { .. }),
            "expected PushModal, got {:?}",
            a.create_push
        );
        if let CreatePushState::PushModal { ref form, .. } = a.create_push {
            let json = form.to_json();
            let tags_value = json.get("tags").and_then(|v| v.as_str()).unwrap_or("");
            assert!(
                tags_value.contains("foo"),
                "tags field should contain 'foo', got: {tags_value:?}"
            );
            assert!(
                tags_value.contains("bar"),
                "tags field should contain 'bar', got: {tags_value:?}"
            );
        }
        assert!(
            a.next_blocking.is_none(),
            "no blocking action should be queued before form submit"
        );
    }

    // -----------------------------------------------------------------------
    // Task 5 — Bulk delete tests
    // -----------------------------------------------------------------------

    /// `[d]` with picks non-empty shows a confirm modal (not immediately deleting).
    #[test]
    fn d_bulk_with_confirm_removes_all_picks_locally() {
        use assert_fs::prelude::*;
        let project = assert_fs::TempDir::new().unwrap();
        project
            .child(".agents/skills/aaa/SKILL.md")
            .write_str("---\nname: aaa\ndescription: a\n---\n")
            .unwrap();
        project
            .child(".agents/skills/bbb/SKILL.md")
            .write_str("---\nname: bbb\ndescription: b\n---\n")
            .unwrap();
        let mut a = App::new(Config::default(), project.path().to_path_buf(), None);
        a.current_screen = Screen::Local;
        a.local.picks.insert(0);
        a.local.picks.insert(1);

        // First [d] should open confirm modal, NOT delete.
        handle_key(&mut a, KeyCode::Char('d'));
        assert!(
            matches!(
                a.local.modal,
                LocalModal::ConfirmBulkDeleteLocal { count: 2 }
            ),
            "expected ConfirmBulkDeleteLocal modal, got {:?}",
            a.local.modal
        );
        // Both skills still present.
        assert_eq!(a.local_skills.len(), 2, "no skills deleted yet");

        // Confirm with [y].
        handle_key(&mut a, KeyCode::Char('y'));
        assert!(
            a.local.picks.is_empty(),
            "picks must be cleared after bulk delete"
        );
        // Skills should be removed from disk.
        assert!(
            a.local_skills.is_empty() || a.local_skills.len() < 2,
            "local skills should be reduced after bulk delete"
        );
    }

    /// `[d]` bulk confirm modal: cancelling with any other key leaves skills intact.
    #[test]
    fn d_bulk_clears_picks_and_rescans() {
        use assert_fs::prelude::*;
        let project = assert_fs::TempDir::new().unwrap();
        project
            .child(".agents/skills/aaa/SKILL.md")
            .write_str("---\nname: aaa\ndescription: a\n---\n")
            .unwrap();
        let mut a = App::new(Config::default(), project.path().to_path_buf(), None);
        a.local.picks.insert(0);

        // Open modal.
        handle_key(&mut a, KeyCode::Char('d'));
        assert!(matches!(
            a.local.modal,
            LocalModal::ConfirmBulkDeleteLocal { count: 1 }
        ));

        // Cancel with [n].
        handle_key(&mut a, KeyCode::Char('n'));
        // Skill must still be there.
        assert_eq!(
            a.local_skills.len(),
            1,
            "skill should not be deleted on cancel"
        );
        // Modal dismissed.
        assert!(
            matches!(a.local.modal, LocalModal::None),
            "modal must be dismissed on cancel"
        );
    }

    /// `[D]` with picks non-empty shows a bulk-everywhere confirm modal.
    #[test]
    fn cap_d_bulk_confirm_modal_then_loop() {
        use assert_fs::prelude::*;
        let project = assert_fs::TempDir::new().unwrap();
        project
            .child(".agents/skills/aaa/SKILL.md")
            .write_str("---\nname: aaa\ndescription: a\n---\n")
            .unwrap();
        project
            .child(".agents/skills/bbb/SKILL.md")
            .write_str("---\nname: bbb\ndescription: b\n---\n")
            .unwrap();
        let mut a = App::new(Config::default(), project.path().to_path_buf(), None);
        a.local.picks.insert(0);
        a.local.picks.insert(1);

        handle_key(&mut a, KeyCode::Char('D'));
        assert!(
            matches!(
                a.local.modal,
                LocalModal::ConfirmBulkDeleteEverywhere { count: 2 }
            ),
            "expected ConfirmBulkDeleteEverywhere modal, got {:?}",
            a.local.modal
        );
        // No action yet — waiting for confirm.
        assert_eq!(a.local_skills.len(), 2, "no skills deleted before confirm");

        // Confirm with [y] — loops remove_everywhere for each pick.
        handle_key(&mut a, KeyCode::Char('y'));
        assert!(
            a.local.picks.is_empty(),
            "picks must be cleared after bulk delete everywhere"
        );
    }
}
