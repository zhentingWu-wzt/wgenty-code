use crate::tui::client::SessionInfo;
use crate::tui::theme;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph};
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

    pub fn delete_selected(&mut self) -> Option<String> {
        if self.sessions.is_empty() {
            return None;
        }
        let id = self.sessions[self.selected].id.clone();
        self.sessions.remove(self.selected);
        if self.selected >= self.sessions.len() && !self.sessions.is_empty() {
            self.selected = self.sessions.len() - 1;
        }
        Some(id)
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
    let popup_area = centered_rect_fn(72, 60, area);
    f.render_widget(Clear, popup_area);

    // Build session list items
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
            // Short id suffix disambiguates sessions that share the same
            // auto-name (first user message). Full UUID is too long for the row.
            let id_short = short_session_id(&s.id);
            let created = format_timestamp(&s.created_at);
            let updated = format_timestamp(&s.updated_at);
            let mut line = format!(
                "{}{}  #{}  {} msgs  created {}  updated {}",
                prefix, name, id_short, s.message_count, created, updated
            );
            // Append summary if present, truncated to fit
            if let Some(summary) = &s.summary {
                let max_summary_len = 50;
                let summary_trunc: String = if summary.len() > max_summary_len {
                    format!(
                        "{}…",
                        summary.chars().take(max_summary_len).collect::<String>()
                    )
                } else {
                    summary.clone()
                };
                line.push_str(&format!("  ─ {}", summary_trunc));
            }
            ListItem::new(line)
        })
        .collect();

    let count_str = if state.sessions.is_empty() {
        "(empty)".to_string()
    } else {
        state.sessions.len().to_string()
    };
    let title = format!(" Sessions ({}) ", count_str);

    // Footer with shortcut hints
    let footer = Paragraph::new(Line::from(vec![
        Span::styled(" ↑↓", Style::default().fg(Color::Cyan)),
        Span::raw(" nav  "),
        Span::styled("Enter", Style::default().fg(Color::Cyan)),
        Span::raw(" load  "),
        Span::styled("d", Style::default().fg(Color::Red)),
        Span::raw(" delete  "),
        Span::styled("Esc", Style::default().fg(Color::Cyan)),
        Span::raw(" close"),
    ]))
    .alignment(Alignment::Center)
    .block(
        Block::default()
            .borders(Borders::TOP)
            .border_style(Style::default().fg(theme::PRIMARY)),
    );

    // Layout: list on top, footer at bottom
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(3)])
        .split(popup_area);

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::TOP | Borders::LEFT | Borders::RIGHT)
            .border_style(Style::default().fg(theme::PRIMARY))
            .title(title),
    );

    // Stateful rendering lets ratatui track the selected index and
    // auto-scroll the viewport so the cursor stays visible.
    let mut list_state = ListState::default();
    if !state.sessions.is_empty() {
        list_state.select(Some(state.selected));
    }

    f.render_stateful_widget(list, chunks[0], &mut list_state);
    f.render_widget(footer, chunks[1]);
}

/// Format an ISO timestamp to a compact display format: "MM/DD HH:MM"
fn format_timestamp(iso: &str) -> String {
    // ISO 8601 format: "2026-06-01T14:30:00..." → "06/01 14:30"
    if iso.len() >= 16 {
        format!("{}/{} {}", &iso[5..7], &iso[8..10], &iso[11..16])
    } else if iso.len() >= 10 {
        iso[..10].to_string()
    } else {
        iso.to_string()
    }
}

/// First 8 chars of a session id (UUID prefix or raw id head).
fn short_session_id(id: &str) -> &str {
    if id.is_empty() {
        return id;
    }
    // Prefer the UUID's first segment when present.
    if let Some(head) = id.split('-').next() {
        if head.len() >= 8 {
            return &id[..8];
        }
    }
    let end = id.char_indices().nth(8).map(|(i, _)| i).unwrap_or(id.len());
    &id[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_session_id_uses_uuid_prefix() {
        assert_eq!(
            short_session_id("abcdef12-3456-7890-abcd-ef1234567890"),
            "abcdef12"
        );
    }

    #[test]
    fn short_session_id_handles_short_ids() {
        assert_eq!(short_session_id("abc"), "abc");
        assert_eq!(short_session_id(""), "");
    }
}
