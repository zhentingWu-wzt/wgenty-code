use crate::tui::client::SessionInfo;
use crate::tui::theme;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::{Block, Borders, Clear, List, ListItem};
use ratatui::Frame;

pub struct SessionState {
    pub visible: bool,
    pub sessions: Vec<SessionInfo>,
    pub selected: usize,
    pub search_query: String,
}

impl SessionState {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            visible: false,
            sessions: Vec::new(),
            selected: 0,
            search_query: String::new(),
        }
    }

    pub fn show(&mut self, sessions: Vec<SessionInfo>) {
        self.visible = true;
        self.sessions = sessions;
        self.selected = 0;
    }

    pub fn dismiss(&mut self) {
        self.visible = false;
    }

    pub fn move_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn move_down(&mut self) {
        if !self.sessions.is_empty() {
            self.selected = (self.selected + 1).min(self.sessions.len() - 1);
        }
    }

    pub fn selected_session(&self) -> Option<&SessionInfo> {
        self.sessions.get(self.selected)
    }
}

/// Render session list popup.
pub fn render(
    f: &mut Frame,
    state: &SessionState,
    centered_rect_fn: impl Fn(u16, u16, Rect) -> Rect,
) {
    if !state.visible {
        return;
    }

    let area = f.area();
    let popup_area = centered_rect_fn(60, 50, area);
    f.render_widget(Clear, popup_area);

    let items: Vec<ListItem> = state
        .sessions
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let prefix = if i == state.selected { "▶ " } else { "  " };
            let name = if s.name.is_empty() {
                "(unnamed)"
            } else {
                &s.name
            };
            ListItem::new(format!(
                "{}{}  ({} msgs, {})",
                prefix,
                name,
                s.message_count,
                &s.updated_at[..s.updated_at.len().min(16)]
            ))
        })
        .collect();

    let title = if state.search_query.is_empty() {
        " Sessions (Ctrl+S toggle, ↑↓ select, Enter load, / search) ".to_string()
    } else {
        format!(" Sessions — search: {} ", state.search_query)
    };

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme::PRIMARY))
            .title(title),
    );

    f.render_widget(list, popup_area);
}
