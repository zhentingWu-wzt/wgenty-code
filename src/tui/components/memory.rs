//! Memory browser popup — list project/global memories (hygiene L1).

use crate::context::{MemoryEntry, MemoryOrigin, MemoryType};
use crate::tui::theme;
use chrono::{DateTime, Utc};
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Which pool is shown in the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryFilter {
    All,
    Project,
    Global,
}

impl MemoryFilter {
    pub fn label(self) -> &'static str {
        match self {
            MemoryFilter::All => "All",
            MemoryFilter::Project => "Project",
            MemoryFilter::Global => "Global",
        }
    }

    pub fn next(self) -> Self {
        match self {
            MemoryFilter::All => MemoryFilter::Project,
            MemoryFilter::Project => MemoryFilter::Global,
            MemoryFilter::Global => MemoryFilter::All,
        }
    }
}

/// One row in the memory browser (origin retained for display/filter).
#[derive(Debug, Clone)]
pub struct MemoryListItem {
    pub origin: MemoryOrigin,
    pub entry: MemoryEntry,
}

pub struct MemoryState {
    pub visible: bool,
    /// Full list as loaded (both pools).
    pub items: Vec<MemoryListItem>,
    pub selected: usize,
    pub filter: MemoryFilter,
    /// When true, show full content pane for the selected item.
    pub detail_mode: bool,
    pub loading: bool,
    pub status_line: String,
    /// When true, a delete is pending confirmation for the selected row.
    pub pending_delete: bool,
}

impl MemoryState {
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            visible: false,
            items: Vec::new(),
            selected: 0,
            filter: MemoryFilter::All,
            detail_mode: false,
            loading: false,
            status_line: String::new(),
            pending_delete: false,
        }
    }

    pub fn show_loading(&mut self) {
        self.visible = true;
        self.loading = true;
        self.detail_mode = false;
        self.pending_delete = false;
        self.items.clear();
        self.selected = 0;
        self.status_line = "Loading…".to_string();
    }

    pub fn show_items(&mut self, items: Vec<MemoryListItem>) {
        self.visible = true;
        self.loading = false;
        self.items = items;
        self.selected = 0;
        self.detail_mode = false;
        self.pending_delete = false;
        let (p, g) = self.pool_counts();
        self.status_line = format!("project {p} · global {g}");
    }

    pub fn dismiss(&mut self) {
        self.visible = false;
        self.detail_mode = false;
        self.loading = false;
        self.pending_delete = false;
    }

    pub fn pool_counts(&self) -> (usize, usize) {
        let mut p = 0usize;
        let mut g = 0usize;
        for item in &self.items {
            match item.origin {
                MemoryOrigin::Project => p += 1,
                MemoryOrigin::Global => g += 1,
            }
        }
        (p, g)
    }

    pub fn filtered_indices(&self) -> Vec<usize> {
        self.items
            .iter()
            .enumerate()
            .filter(|(_, item)| match self.filter {
                MemoryFilter::All => true,
                MemoryFilter::Project => item.origin == MemoryOrigin::Project,
                MemoryFilter::Global => item.origin == MemoryOrigin::Global,
            })
            .map(|(i, _)| i)
            .collect()
    }

    pub fn move_up(&mut self) {
        let n = self.filtered_indices().len();
        if n == 0 {
            return;
        }
        self.selected = if self.selected == 0 {
            n - 1
        } else {
            self.selected - 1
        };
    }

    pub fn move_down(&mut self) {
        let n = self.filtered_indices().len();
        if n == 0 {
            return;
        }
        self.selected = (self.selected + 1) % n;
    }

    pub fn cycle_filter(&mut self) {
        self.filter = self.filter.next();
        self.selected = 0;
        self.detail_mode = false;
        self.pending_delete = false;
    }

    pub fn selected_item(&self) -> Option<&MemoryListItem> {
        let indices = self.filtered_indices();
        indices.get(self.selected).and_then(|&i| self.items.get(i))
    }

    pub fn toggle_detail(&mut self) {
        if self.selected_item().is_some() {
            self.detail_mode = !self.detail_mode;
        }
    }

    /// Arm the delete-confirmation state for the selected row. No-op if
    /// nothing is selected or a delete is already pending.
    pub fn request_delete(&mut self) {
        if self.selected_item().is_some() {
            self.pending_delete = true;
        }
    }

    /// Cancel a pending delete (called on any non-confirming key).
    pub fn cancel_delete(&mut self) {
        self.pending_delete = false;
    }

    /// Confirm the pending delete: returns `(origin, id)` of the selected
    /// row so the caller can fire the async `DeleteMemory` event. Clears
    /// `pending_delete` regardless of outcome.
    pub fn confirm_delete(&mut self) -> Option<(MemoryOrigin, String)> {
        if !self.pending_delete {
            return None;
        }
        self.pending_delete = false;
        self.selected_item()
            .map(|item| (item.origin, item.entry.id.clone()))
    }
}

/// Render memory browser popup (session-style overlay).
pub fn render(
    f: &mut Frame,
    state: &MemoryState,
    centered_rect_fn: impl Fn(u16, u16, Rect) -> Rect,
) {
    if !state.visible {
        return;
    }

    let area = f.area();
    let popup_area = centered_rect_fn(80, 70, area);
    f.render_widget(Clear, popup_area);

    let indices = state.filtered_indices();
    let (project_n, global_n) = state.pool_counts();

    let title = format!(
        " Memories · {} ({}/{}) · p:{} g:{} ",
        state.filter.label(),
        if indices.is_empty() {
            0
        } else {
            state.selected + 1
        },
        indices.len(),
        project_n,
        global_n
    );

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),
            Constraint::Length(if state.detail_mode { 10 } else { 0 }),
            Constraint::Length(3),
        ])
        .split(popup_area);

    // Inner width of the list pane (exclude L/R borders).
    let list_inner_width = chunks[0].width.saturating_sub(2) as usize;

    // List
    let items: Vec<ListItem> = if state.loading {
        vec![ListItem::new("  Loading memories…")]
    } else if indices.is_empty() {
        vec![ListItem::new("  (no memories in this filter)")]
    } else {
        indices
            .iter()
            .enumerate()
            .map(|(view_i, &src_i)| {
                let item = &state.items[src_i];
                let marker = if view_i == state.selected {
                    "▶ "
                } else {
                    "  "
                };
                let origin = match item.origin {
                    MemoryOrigin::Project => "proj",
                    MemoryOrigin::Global => "glob",
                };
                let ty = memory_type_label(&item.entry.memory_type);
                let age = format_age(item.entry.timestamp);
                // Keep metadata fixed; fill the remaining row width with content.
                let prefix = format!(
                    "{}★{:.2}  {:4}  {:10}  {:>4}  ",
                    marker, item.entry.importance, origin, ty, age
                );
                let preview_max = list_inner_width.saturating_sub(prefix.width());
                let preview =
                    truncate_to_width(&item.entry.content.replace('\n', " "), preview_max);
                let line = format!("{prefix}{preview}");
                let style = if view_i == state.selected {
                    Style::default()
                        .fg(Color::Rgb(203, 166, 247))
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::Rgb(205, 205, 220))
                };
                ListItem::new(Line::from(Span::styled(line, style)))
            })
            .collect()
    };

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::TOP | Borders::LEFT | Borders::RIGHT)
            .border_style(Style::default().fg(theme::PRIMARY))
            .title(title),
    );

    let mut list_state = ListState::default();
    if !indices.is_empty() {
        list_state.select(Some(state.selected.min(indices.len() - 1)));
    }
    f.render_stateful_widget(list, chunks[0], &mut list_state);

    // Detail pane
    if state.detail_mode {
        if let Some(item) = state.selected_item() {
            let origin = match item.origin {
                MemoryOrigin::Project => "project",
                MemoryOrigin::Global => "global",
            };
            let tags = if item.entry.tags.is_empty() {
                "—".to_string()
            } else {
                item.entry.tags.join(", ")
            };
            let header = format!(
                "id {} · {} · {} · ★{:.2} · {}\ntags: {}",
                short_id(&item.entry.id),
                origin,
                memory_type_label(&item.entry.memory_type),
                item.entry.importance,
                item.entry.timestamp.format("%Y-%m-%d %H:%M UTC"),
                tags
            );
            let body = format!("{}\n\n{}", header, item.entry.content);
            let detail = Paragraph::new(body).wrap(Wrap { trim: false }).block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(theme::PRIMARY))
                    .title(" Detail "),
            );
            f.render_widget(detail, chunks[1]);
        }
    }

    // Footer - changes when a delete is pending confirmation.
    let footer_spans: Vec<Span> = if state.pending_delete {
        vec![
            Span::styled(" ⚠ Delete this memory?", Style::default().fg(Color::Yellow)),
            Span::raw("  "),
            Span::styled(
                "y",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" confirm  "),
            Span::styled("n/Esc", Style::default().fg(Color::Cyan)),
            Span::raw(" cancel  "),
            Span::styled(
                state.status_line.clone(),
                Style::default().fg(Color::Rgb(108, 112, 134)),
            ),
        ]
    } else {
        vec![
            Span::styled(" ↑↓", Style::default().fg(Color::Cyan)),
            Span::raw(" nav  "),
            Span::styled("Tab", Style::default().fg(Color::Cyan)),
            Span::raw(" filter  "),
            Span::styled("Enter", Style::default().fg(Color::Cyan)),
            Span::raw(" detail  "),
            Span::styled("d", Style::default().fg(Color::Red)),
            Span::raw(" delete  "),
            Span::styled("Esc", Style::default().fg(Color::Cyan)),
            Span::raw(" close  "),
            Span::styled(
                state.status_line.clone(),
                Style::default().fg(Color::Rgb(108, 112, 134)),
            ),
        ]
    };

    let footer = Paragraph::new(Line::from(footer_spans))
        .alignment(Alignment::Left)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme::PRIMARY)),
        );
    f.render_widget(footer, chunks[2]);
}

fn memory_type_label(t: &MemoryType) -> &'static str {
    match t {
        MemoryType::Session => "Session",
        MemoryType::Conversation => "Convers.",
        MemoryType::Knowledge => "Knowledge",
        MemoryType::Preference => "Prefer.",
        MemoryType::Task => "Task",
        MemoryType::Error => "Error",
        MemoryType::Insight => "Insight",
        MemoryType::Decision => "Decision",
    }
}

fn short_id(id: &str) -> String {
    id.chars().take(8).collect()
}

/// Truncate `s` to at most `max_width` terminal columns, appending `…` when cut.
/// Uses Unicode display width so CJK / wide glyphs don't overshoot the panel.
fn truncate_to_width(s: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    if s.width() <= max_width {
        return s.to_string();
    }
    let ellipsis = '…';
    let ellipsis_w = ellipsis.width().unwrap_or(1);
    if max_width < ellipsis_w {
        return String::new();
    }
    let target = max_width - ellipsis_w;
    let mut out = String::new();
    let mut used = 0usize;
    for ch in s.chars() {
        let w = ch.width().unwrap_or(0);
        if used + w > target {
            break;
        }
        out.push(ch);
        used += w;
    }
    out.push(ellipsis);
    out
}

fn format_age(ts: DateTime<Utc>) -> String {
    let now = Utc::now();
    let delta = now.signed_duration_since(ts);
    if delta.num_seconds() < 0 {
        return "now".into();
    }
    if delta.num_days() >= 1 {
        format!("{}d", delta.num_days())
    } else if delta.num_hours() >= 1 {
        format!("{}h", delta.num_hours())
    } else if delta.num_minutes() >= 1 {
        format!("{}m", delta.num_minutes())
    } else {
        "now".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(origin: MemoryOrigin, content: &str, importance: f32) -> MemoryListItem {
        let mut entry = MemoryEntry::new(MemoryType::Knowledge, content);
        entry.importance = importance;
        MemoryListItem { origin, entry }
    }

    #[test]
    fn filter_cycles_and_filters_pools() {
        let mut state = MemoryState::new();
        state.show_items(vec![
            item(MemoryOrigin::Project, "p1", 0.9),
            item(MemoryOrigin::Global, "g1", 0.8),
            item(MemoryOrigin::Project, "p2", 0.5),
        ]);
        assert_eq!(state.filtered_indices().len(), 3);
        state.cycle_filter(); // Project
        assert_eq!(state.filter, MemoryFilter::Project);
        assert_eq!(state.filtered_indices().len(), 2);
        state.cycle_filter(); // Global
        assert_eq!(state.filtered_indices().len(), 1);
        state.cycle_filter(); // All
        assert_eq!(state.filtered_indices().len(), 3);
    }

    #[test]
    fn selection_wraps() {
        let mut state = MemoryState::new();
        state.show_items(vec![
            item(MemoryOrigin::Project, "a", 0.5),
            item(MemoryOrigin::Project, "b", 0.5),
        ]);
        state.move_down();
        assert_eq!(state.selected, 1);
        state.move_down();
        assert_eq!(state.selected, 0);
        state.move_up();
        assert_eq!(state.selected, 1);
    }

    #[test]
    fn detail_toggles_only_with_selection() {
        let mut state = MemoryState::new();
        state.show_items(vec![]);
        state.toggle_detail();
        assert!(!state.detail_mode);
        state.show_items(vec![item(MemoryOrigin::Global, "x", 0.1)]);
        state.toggle_detail();
        assert!(state.detail_mode);
    }

    #[test]
    fn truncate_to_width_fits_ascii_and_cjk() {
        assert_eq!(truncate_to_width("hello", 10), "hello");
        assert_eq!(truncate_to_width("hello world", 8), "hello w…");
        // CJK glyphs are typically width 2; max 5 => one char + ellipsis.
        let cjk = truncate_to_width("中文内容测试", 5);
        assert!(cjk.ends_with('…'));
        assert!(cjk.width() <= 5);
        assert_eq!(truncate_to_width("abc", 0), "");
    }

    #[test]
    fn delete_confirmation_flow() {
        let mut state = MemoryState::new();
        // No selection -> request_delete is a no-op.
        state.request_delete();
        assert!(!state.pending_delete);

        state.show_items(vec![
            item(MemoryOrigin::Project, "a", 0.5),
            item(MemoryOrigin::Global, "b", 0.8),
        ]);

        // Arm delete.
        state.request_delete();
        assert!(state.pending_delete);

        // Cancel clears the flag without consuming the selection.
        state.cancel_delete();
        assert!(!state.pending_delete);

        // Re-arm, then confirm returns the selected item's (origin, id).
        state.request_delete();
        let confirmed = state.confirm_delete();
        assert!(confirmed.is_some());
        let (origin, id) = confirmed.unwrap();
        assert_eq!(origin, MemoryOrigin::Project);
        assert!(!id.is_empty());
        assert!(!state.pending_delete);

        // Confirm with nothing pending returns None.
        assert!(state.confirm_delete().is_none());
    }

    #[test]
    fn cycle_filter_cancels_pending_delete() {
        let mut state = MemoryState::new();
        state.show_items(vec![item(MemoryOrigin::Project, "a", 0.5)]);
        state.request_delete();
        assert!(state.pending_delete);
        state.cycle_filter();
        assert!(!state.pending_delete);
    }
}
