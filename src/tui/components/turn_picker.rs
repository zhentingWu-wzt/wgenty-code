//! Turn-picker popup component for the `/undo` interactive rollback flow.
//!
//! Lists recorded turns (number / timestamp / first-sentence summary / file
//! count) and lets the user navigate with ↑/↓ and confirm with Enter or cancel
//! with Esc.  The caller is expected to pre-filter `turns` so that only
//! undo-able entries (those before the compaction boundary) are shown.

use crate::context::TurnRecord;
use crate::tui::theme;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState};
use ratatui::Frame;

/// Mutable state backing the turn-picker popup.
///
/// `turns` should already be filtered to exclude everything after the
/// compaction boundary.  When the list is empty the popup is created in the
/// closed state (`open = false`) so an empty list is never displayed.
#[derive(Debug, Clone)]
pub struct TurnPickerState {
    /// Turns available for selection (pre-filtered past compaction boundary).
    pub turns: Vec<TurnRecord>,
    /// Index of the currently highlighted turn.
    pub selected: usize,
    /// Whether the popup is currently visible.
    pub open: bool,
}

impl TurnPickerState {
    /// Create a new picker from the given (pre-filtered) turns.
    ///
    /// `selected` starts at 0.  When `turns` is empty, `open` is set to
    /// `false` so the popup never displays an empty list.
    pub fn new(turns: Vec<TurnRecord>) -> Self {
        let open = !turns.is_empty();
        Self {
            turns,
            selected: 0,
            open,
        }
    }

    /// Move the selection up by one (clamped at 0, no wrap-around).
    pub fn up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    /// Move the selection down by one (clamped at last index, no wrap-around).
    pub fn down(&mut self) {
        if !self.turns.is_empty() {
            self.selected = (self.selected + 1).min(self.turns.len() - 1);
        }
    }

    /// Return the `turn_id` of the currently selected turn, or `None` when
    /// the list is empty.
    pub fn selected_turn_id(&self) -> Option<&str> {
        self.turns.get(self.selected).map(|t| t.turn_id.as_str())
    }

    /// Close the popup (sets `open` to `false`).
    pub fn close(&mut self) {
        self.open = false;
    }
}

/// Render the turn-picker popup into `area`.
///
/// The caller is responsible for computing a centered `area` (typically via
/// [`crate::tui::util::centered_rect`]).  When `state.open` is `false` this
/// is a no-op.
//
// Not wired into the app render pipeline yet — integrated in Task 8.
#[allow(dead_code)]
pub fn render(f: &mut Frame, state: &TurnPickerState, area: Rect) {
    if !state.open {
        return;
    }

    f.render_widget(Clear, area);

    // Build list items: "#idx  time  summary  ✓N" (idx is 1-based).
    let items: Vec<ListItem> = state
        .turns
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let idx = i + 1;
            let time = format_timestamp(&t.created_at);
            let files = if t.file_count > 0 {
                format!(" ✓{}", t.file_count)
            } else {
                String::new()
            };
            let line = format!("#{}  {}  {}{}", idx, time, t.user_summary, files);
            ListItem::new(line)
        })
        .collect();

    let title = format!(" Undo to turn ({}) ", state.turns.len());

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme::PRIMARY))
                .title(title),
        )
        .highlight_style(
            Style::default()
                .fg(theme::ACCENT)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");

    // Stateful rendering so ratatui tracks the selected index and
    // auto-scrolls the viewport to keep the cursor visible.
    let mut list_state = ListState::default();
    if !state.turns.is_empty() {
        list_state.select(Some(state.selected));
    }

    f.render_stateful_widget(list, area, &mut list_state);
}

/// Format an ISO-8601 timestamp to a compact display: "MM/DD HH:MM".
//
// Only called from `render`; kept private.  Marked dead_code because `render`
// itself is not wired in until Task 8.
#[allow(dead_code)]
fn format_timestamp(iso: &str) -> String {
    // ISO 8601: "2025-06-01T14:30:00..." -> "06/01 14:30"
    if iso.len() >= 16 {
        format!("{}/{} {}", &iso[5..7], &iso[8..10], &iso[11..16])
    } else if iso.len() >= 10 {
        iso[..10].to_string()
    } else {
        iso.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::TurnRecord;

    /// Helper: build a `TurnRecord` with minimal fields for testing.
    fn make_turn(id: &str, files: usize) -> TurnRecord {
        TurnRecord {
            turn_id: id.to_string(),
            created_at: "2025-06-01T14:30:00Z".to_string(),
            user_summary: format!("Turn {}", id),
            checkpoint_turn_id: String::new(),
            message_end_idx: 0,
            file_count: files,
            committed_messages_end_idx: 0,
        }
    }

    #[test]
    fn new_empty_turns_is_closed() {
        let state = TurnPickerState::new(vec![]);
        assert!(!state.open);
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn new_with_turns_is_open_at_zero() {
        let state = TurnPickerState::new(vec![
            make_turn("a", 0),
            make_turn("b", 1),
            make_turn("c", 2),
        ]);
        assert!(state.open);
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn down_increments_and_clamps_at_end() {
        let mut state = TurnPickerState::new(vec![
            make_turn("a", 0),
            make_turn("b", 0),
            make_turn("c", 0),
        ]);
        assert_eq!(state.selected, 0);
        state.down();
        assert_eq!(state.selected, 1);
        state.down();
        assert_eq!(state.selected, 2);
        // Clamp at last index - no wrap-around.
        state.down();
        assert_eq!(state.selected, 2);
    }

    #[test]
    fn up_decrements_and_clamps_at_start() {
        let mut state = TurnPickerState::new(vec![
            make_turn("a", 0),
            make_turn("b", 0),
            make_turn("c", 0),
        ]);
        state.selected = 2;
        state.up();
        assert_eq!(state.selected, 1);
        state.up();
        assert_eq!(state.selected, 0);
        // Clamp at 0 - no wrap-around.
        state.up();
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn selected_turn_id_returns_current() {
        let mut state = TurnPickerState::new(vec![
            make_turn("a", 0),
            make_turn("b", 0),
            make_turn("c", 0),
        ]);
        assert_eq!(state.selected_turn_id(), Some("a"));
        state.down();
        assert_eq!(state.selected_turn_id(), Some("b"));
    }

    #[test]
    fn selected_turn_id_none_when_empty() {
        let state = TurnPickerState::new(vec![]);
        assert_eq!(state.selected_turn_id(), None);
    }

    #[test]
    fn close_sets_open_false() {
        let mut state = TurnPickerState::new(vec![make_turn("a", 0)]);
        assert!(state.open);
        state.close();
        assert!(!state.open);
    }
}
