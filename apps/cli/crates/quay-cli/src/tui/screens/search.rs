use crate::tui::app::{App, ScreenAction};
use crate::tui::theme;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::Frame;

#[derive(Debug, Default)]
pub struct SearchState {
    pub query: String,
    pub all_skills: Vec<SkillRow>,
    pub filtered: Vec<usize>,
    pub list_state: ListState,
    pub fetched: bool,
}

#[derive(Debug, Clone)]
pub struct SkillRow {
    pub remote: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub tags: Vec<String>,
}

pub fn handle_key(app: &mut App, code: KeyCode) -> ScreenAction {
    match code {
        KeyCode::Down => {
            let max = app.search.filtered.len().saturating_sub(1);
            let i = app.search.list_state.selected().unwrap_or(0);
            app.search
                .list_state
                .select(Some(i.saturating_add(1).min(max)));
            return ScreenAction::Stay;
        }
        KeyCode::Up => {
            let i = app.search.list_state.selected().unwrap_or(0);
            app.search.list_state.select(Some(i.saturating_sub(1)));
            return ScreenAction::Stay;
        }
        _ => {}
    }
    let key_event = KeyEvent {
        code,
        modifiers: KeyModifiers::NONE,
        kind: KeyEventKind::Press,
        state: KeyEventState::empty(),
    };
    app.search_textarea.input(key_event);
    let q = app.search_textarea.lines().join("");
    app.search.query = q;
    refilter(&mut app.search);
    ScreenAction::Stay
}

pub fn refilter(state: &mut SearchState) {
    use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
    use nucleo_matcher::{Matcher, Utf32Str};

    if state.query.is_empty() {
        state.filtered = (0..state.all_skills.len()).collect();
        if !state.filtered.is_empty() {
            state.list_state.select(Some(0));
        }
        return;
    }
    let mut matcher = Matcher::new(nucleo_matcher::Config::DEFAULT);
    let pattern = Pattern::parse(&state.query, CaseMatching::Smart, Normalization::Smart);
    let mut scored: Vec<(usize, u32)> = Vec::new();
    let mut buf = Vec::new();
    for (idx, sk) in state.all_skills.iter().enumerate() {
        let haystack = format!("{} {}", sk.name, sk.description);
        let haystack_utf32 = Utf32Str::new(&haystack, &mut buf);
        if let Some(score) = pattern.score(haystack_utf32, &mut matcher) {
            scored.push((idx, score));
        }
    }
    scored.sort_by(|a, b| b.1.cmp(&a.1));
    state.filtered = scored.into_iter().map(|(i, _)| i).collect();
    state.list_state.select(if state.filtered.is_empty() {
        None
    } else {
        Some(0)
    });
}

fn load_all_remotes(app: &App) -> Vec<SkillRow> {
    use quay_core::{search, GithubRawFetcher, SearchFilters};
    let branch = std::env::var("QUAY_GITHUB_BRANCH").unwrap_or_else(|_| "main".into());
    let f = GithubRawFetcher::new(branch);
    let hits = search(&app.cfg, &f, &SearchFilters::default()).unwrap_or_default();
    hits.into_iter()
        .map(|h| SkillRow {
            remote: h.remote,
            name: h.name,
            description: h.description,
            version: h.version,
            tags: h.tags,
        })
        .collect()
}

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    // Render whatever is currently in app.search — actual fetching happens via
    // ensure_loaded which is called on screen entry (from the event loop).

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(1)])
        .split(area);

    frame.render_widget(&app.search_textarea, rows[0]);

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(rows[1]);

    let items: Vec<ListItem> = app
        .search
        .filtered
        .iter()
        .filter_map(|i| app.search.all_skills.get(*i))
        .map(|sk| {
            ListItem::new(Line::from(format!(
                "{}  {} v{}  — {}",
                sk.remote, sk.name, sk.version, sk.description
            )))
        })
        .collect();
    let mut list_state = app.search.list_state.clone();
    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!(" Results ({}) ", app.search.filtered.len())),
        )
        .highlight_style(theme::selected());
    frame.render_stateful_widget(list, cols[0], &mut list_state);

    let preview = app
        .search
        .list_state
        .selected()
        .and_then(|i| app.search.filtered.get(i).copied())
        .and_then(|i| app.search.all_skills.get(i))
        .map(|sk| {
            format!(
                "{}/{}  v{}\ntags: {}\n\n{}",
                sk.remote,
                sk.name,
                sk.version,
                if sk.tags.is_empty() {
                    "(none)".into()
                } else {
                    sk.tags.join(", ")
                },
                sk.description
            )
        })
        .unwrap_or_else(|| "(no selection)".into());
    frame.render_widget(
        Paragraph::new(preview).block(Block::default().borders(Borders::ALL).title(" Preview ")),
        cols[1],
    );
}

/// Triggered on screen entry. Fetches all skills from configured remotes if not
/// already done. Keeps `render`'s `&App` signature clean.
pub fn ensure_loaded(app: &mut App) {
    if !app.search.fetched {
        app.search.all_skills = load_all_remotes(app);
        app.search.filtered = (0..app.search.all_skills.len()).collect();
        app.search.fetched = true;
        if !app.search.filtered.is_empty() {
            app.search.list_state.select(Some(0));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quay_core::{Config, Lockfile};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn empty_app() -> App {
        let mut a = App::new(
            Config::default(),
            Lockfile::default(),
            std::path::PathBuf::from("/tmp"),
            None,
        );
        a.current_screen = crate::tui::app::Screen::Search;
        a
    }

    #[test]
    fn search_renders_input_box_and_results_panel() {
        let mut terminal = Terminal::new(TestBackend::new(80, 20)).unwrap();
        let a = empty_app();
        terminal.draw(|f| crate::tui::draw(f, &a)).unwrap();
        let dump: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(dump.contains("search"), "dump: {}", dump);
        assert!(dump.contains("Results"), "dump: {}", dump);
    }

    #[test]
    fn refilter_with_empty_query_returns_all() {
        let mut s = SearchState {
            query: String::new(),
            all_skills: vec![SkillRow {
                remote: "r".into(),
                name: "csv-parse".into(),
                description: "csv".into(),
                version: "0.1.0".into(),
                tags: vec![],
            }],
            ..Default::default()
        };
        refilter(&mut s);
        assert_eq!(s.filtered.len(), 1);
    }
}
