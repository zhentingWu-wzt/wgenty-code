use crate::tui::client::TodoItem;
use crate::tui::theme;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem};
use ratatui::Frame;

pub struct TaskPanelState {
    pub visible: bool,
    pub items: Vec<TodoItem>,
}

impl TaskPanelState {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            visible: false,
            items: Vec::new(),
        }
    }

    pub fn update(&mut self, items: Vec<TodoItem>) {
        self.items = items;
    }

    pub fn toggle(&mut self) {
        self.visible = !self.visible;
    }
}

/// Render the task panel on the right side of the screen.
pub fn render(f: &mut Frame, area: Rect, state: &TaskPanelState) {
    if !state.visible || state.items.is_empty() {
        return;
    }

    let items: Vec<ListItem> = state
        .items
        .iter()
        .map(|item| {
            if let Some(ref meta) = item.subagent {
                // Subagent task: show 🤖 + type + stats
                let dur_str = if meta.duration_ms >= 60_000 {
                    format!("{:.1}m", meta.duration_ms as f64 / 60_000.0)
                } else if meta.duration_ms >= 1_000 {
                    format!("{:.1}s", meta.duration_ms as f64 / 1_000.0)
                } else {
                    format!("{}ms", meta.duration_ms)
                };
                let tokens_str = if meta.token_usage >= 1000 {
                    format!("{:.1}k", meta.token_usage as f64 / 1000.0)
                } else {
                    format!("{}", meta.token_usage)
                };
                let stats = format!(
                    "{} · {}r · {} · {} tokens",
                    meta.subagent_type, meta.rounds, dur_str, tokens_str
                );
                ListItem::new(Line::from(vec![
                    Span::styled(
                        "\u{1f916} ",
                        Style::default().fg(theme::INFO),
                    ),
                    Span::styled(stats, Style::default().fg(theme::DIM)),
                    Span::raw("  "),
                    Span::raw(&item.content),
                ]))
            } else {
                // Regular task: existing rendering
                let (icon, color) = match item.status.as_str() {
                    "completed" => ("\u{2713}", theme::SUCCESS),
                    "in_progress" => ("\u{25cf}", theme::ACCENT),
                    _ => ("\u{25cb}", theme::DIM),
                };
                let label = if item.active_form.is_empty() {
                    &item.content
                } else {
                    &item.active_form
                };
                ListItem::new(Line::from(vec![
                    Span::styled(format!("{} ", icon), Style::default().fg(color)),
                    Span::raw(label),
                ]))
            }
        })
        .collect();

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(theme::DIM))
            .title(" Tasks (Ctrl+T toggle) "),
    );

    f.render_widget(list, area);
}
