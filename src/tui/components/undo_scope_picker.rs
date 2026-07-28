//! Undo-scope picker popup component for the `/undo` interactive rollback flow.
//!
//! Presents three choices - Code, Chat, or Both - and lets the user navigate
//! with ↑/↓ and confirm with Enter or cancel with Esc.  The default selection
//! is `Both`.

use crate::tui::app::turn::UndoScope;
use crate::tui::theme;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState};
use ratatui::Frame;

/// Mutable state backing the undo-scope picker popup.
///
/// `selected` starts at `UndoScope::Both`.  Navigation cycles through
/// Code -> Chat -> Both (and reverse) with wrap-around.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UndoScopePickerState {
    /// Currently highlighted scope.
    pub selected: UndoScope,
    /// Whether the popup is currently visible.
    pub open: bool,
}

impl UndoScopePickerState {
    /// Create a new picker with `selected = Both` and `open = true`.
    pub fn new() -> Self {
        Self {
            selected: UndoScope::Both,
            open: true,
        }
    }

    /// Move the selection up by one with wrap-around
    /// (Both -> Chat -> Code -> Both).
    pub fn up(&mut self) {
        self.selected = match self.selected {
            UndoScope::Code => UndoScope::Both,
            UndoScope::Chat => UndoScope::Code,
            UndoScope::Both => UndoScope::Chat,
        };
    }

    /// Move the selection down by one with wrap-around
    /// (Code -> Chat -> Both -> Code).
    pub fn down(&mut self) {
        self.selected = match self.selected {
            UndoScope::Code => UndoScope::Chat,
            UndoScope::Chat => UndoScope::Both,
            UndoScope::Both => UndoScope::Code,
        };
    }

    /// Return the currently selected [`UndoScope`].
    pub fn selected(&self) -> UndoScope {
        self.selected
    }

    /// Close the popup (sets `open` to `false`).
    pub fn close(&mut self) {
        self.open = false;
    }
}

impl Default for UndoScopePickerState {
    fn default() -> Self {
        Self::new()
    }
}

/// Render the undo-scope picker popup into `area`.
///
/// The caller is responsible for computing a centered `area` (typically via
/// [`crate::tui::util::centered_rect`]).  When `state.open` is `false` this
/// is a no-op.
//
pub fn render(f: &mut Frame, state: &UndoScopePickerState, area: Rect) {
    if !state.open {
        return;
    }

    f.render_widget(Clear, area);

    let items = vec![
        ListItem::new("Code"),
        ListItem::new("Chat"),
        ListItem::new("Both"),
    ];

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme::PRIMARY))
                .title(" Undo scope "),
        )
        .highlight_style(
            Style::default()
                .fg(theme::ACCENT)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");

    let mut list_state = ListState::default();
    list_state.select(Some(scope_index(state.selected)));

    f.render_stateful_widget(list, area, &mut list_state);
}

/// Map an [`UndoScope`] to its 0-based display index (Code=0, Chat=1, Both=2).
//
/// Only called from `render`; kept private.
fn scope_index(scope: UndoScope) -> usize {
    match scope {
        UndoScope::Code => 0,
        UndoScope::Chat => 1,
        UndoScope::Both => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_defaults_to_both_and_open() {
        let state = UndoScopePickerState::new();
        assert_eq!(state.selected, UndoScope::Both);
        assert!(state.open);
    }

    #[test]
    fn down_cycles_both_to_code_to_chat_to_both() {
        let mut state = UndoScopePickerState::new();
        // Both -> Code -> Chat -> Both (wrap-around).
        assert_eq!(state.selected(), UndoScope::Both);
        state.down();
        assert_eq!(state.selected(), UndoScope::Code);
        state.down();
        assert_eq!(state.selected(), UndoScope::Chat);
        state.down();
        assert_eq!(state.selected(), UndoScope::Both);
    }

    #[test]
    fn up_cycles_in_reverse() {
        let mut state = UndoScopePickerState::new();
        // Both -> Chat -> Code -> Both (reverse wrap-around).
        state.up();
        assert_eq!(state.selected(), UndoScope::Chat);
        state.up();
        assert_eq!(state.selected(), UndoScope::Code);
        state.up();
        assert_eq!(state.selected(), UndoScope::Both);
    }

    #[test]
    fn selected_returns_current_scope() {
        let mut state = UndoScopePickerState::new();
        assert_eq!(state.selected(), UndoScope::Both);
        state.down();
        assert_eq!(state.selected(), UndoScope::Code);
        state.down();
        assert_eq!(state.selected(), UndoScope::Chat);
    }

    #[test]
    fn close_sets_open_false() {
        let mut state = UndoScopePickerState::new();
        assert!(state.open);
        state.close();
        assert!(!state.open);
    }
}
