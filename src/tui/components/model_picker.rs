//! Model picker popup component for the `/model` switch flow.
//!
//! Presents the switchable model profiles declared in `settings.json` under
//! `models.profiles` and lets the user navigate with ↑/↓ (or j/k), confirm
//! with Enter, or cancel with Esc.  The currently active profile (if any) is
//! selected by default and marked `●` in the list.
//!
//! The picker holds only display state; the actual switch is performed by the
//! App via `DaemonClient::switch_model` once the user confirms.

use crate::tui::client::ModelOption;
use crate::tui::theme;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState};
use ratatui::Frame;

/// Mutable state backing the model picker popup.
///
/// `options` is the list returned by `GET /api/v1/models`; `selected` is the
/// 0-based index of the highlighted row.  Navigation wraps around.
#[derive(Debug, Clone)]
pub struct ModelPickerState {
    pub options: Vec<ModelOption>,
    pub selected: usize,
    pub open: bool,
}

impl ModelPickerState {
    /// Build a picker from the daemon's model list.  The active profile (if
    /// any) is selected initially; otherwise the first row.
    pub fn new(options: Vec<ModelOption>) -> Self {
        let selected = options
            .iter()
            .position(|o| o.active)
            .unwrap_or(0)
            .min(options.len().saturating_sub(1));
        Self {
            options,
            selected,
            open: true,
        }
    }

    /// Move the selection up by one with wrap-around.
    pub fn up(&mut self) {
        if self.options.is_empty() {
            return;
        }
        self.selected = if self.selected == 0 {
            self.options.len() - 1
        } else {
            self.selected - 1
        };
    }

    /// Move the selection down by one with wrap-around.
    pub fn down(&mut self) {
        if self.options.is_empty() {
            return;
        }
        self.selected = if self.selected + 1 >= self.options.len() {
            0
        } else {
            self.selected + 1
        };
    }

    /// The currently highlighted option, if any.
    pub fn selected_option(&self) -> Option<&ModelOption> {
        self.options.get(self.selected)
    }

    /// Close the popup (sets `open` to `false`).
    pub fn close(&mut self) {
        self.open = false;
    }

    /// True when there are no profiles to pick from.
    pub fn is_empty(&self) -> bool {
        self.options.is_empty()
    }
}

/// Render the model picker popup into `area`.
///
/// The caller computes a centered `area`.  When `state.open` is `false` this is
/// a no-op.  An empty profile list renders a hint telling the user how to
/// declare profiles in `settings.json`.
pub fn render(f: &mut Frame, state: &ModelPickerState, area: Rect) {
    if !state.open {
        return;
    }

    f.render_widget(Clear, area);

    let items: Vec<ListItem> = if state.options.is_empty() {
        vec![ListItem::new(
            "No model profiles configured. Add `models.profiles` in settings.json \
             to enable switching (see WGENTY.md).",
        )]
    } else {
        state
            .options
            .iter()
            .map(|o| {
                let marker = if o.active { "● " } else { "  " };
                let provider = o.provider.as_deref().unwrap_or("");
                // Tier tag (e.g. "light"/"heavy") shown when declared, so the
                // user can see which profiles participate in auto-routing.
                let tier_tag = o
                    .tier
                    .as_deref()
                    .map(|t| format!("  {{tier:{t}}}"))
                    .unwrap_or_default();
                ListItem::new(format!(
                    "{marker}{}  ({})  [{}]{}",
                    o.label, o.model_name, provider, tier_tag
                ))
            })
            .collect()
    };

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme::PRIMARY))
                .title(" Switch model (↑/↓ select, Enter confirm, Esc cancel) "),
        )
        .highlight_style(
            Style::default()
                .fg(theme::ACCENT)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");

    let mut list_state = ListState::default();
    if !state.options.is_empty() {
        list_state.select(Some(state.selected));
    }

    f.render_stateful_widget(list, area, &mut list_state);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opt(key: &str, label: &str, active: bool) -> ModelOption {
        ModelOption {
            key: key.to_string(),
            label: label.to_string(),
            model_name: format!("{key}-model"),
            provider: None,
            tier: None,
            active,
        }
    }

    #[test]
    fn new_selects_active_profile_by_default() {
        let state = ModelPickerState::new(vec![
            opt("fast", "Fast", false),
            opt("smart", "Smart", true),
            opt("cheap", "Cheap", false),
        ]);
        assert_eq!(state.selected, 1);
        assert!(state.open);
        assert_eq!(state.selected_option().unwrap().key, "smart");
    }

    #[test]
    fn new_defaults_to_first_when_none_active() {
        let state = ModelPickerState::new(vec![opt("a", "A", false), opt("b", "B", false)]);
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn down_wraps_around() {
        let mut state = ModelPickerState::new(vec![
            opt("a", "A", false),
            opt("b", "B", false),
            opt("c", "C", true),
        ]);
        assert_eq!(state.selected, 2);
        state.down(); // last -> wrap to 0
        assert_eq!(state.selected, 0);
        state.down();
        assert_eq!(state.selected, 1);
    }

    #[test]
    fn up_wraps_around() {
        let mut state = ModelPickerState::new(vec![opt("a", "A", false), opt("b", "B", true)]);
        assert_eq!(state.selected, 1);
        state.up();
        assert_eq!(state.selected, 0);
        state.up(); // 0 -> wrap to last
        assert_eq!(state.selected, 1);
    }

    #[test]
    fn empty_list_is_safe() {
        let mut state = ModelPickerState::new(vec![]);
        assert!(state.is_empty());
        assert!(state.selected_option().is_none());
        // Navigation on empty list is a no-op (must not panic).
        state.up();
        state.down();
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn close_sets_open_false() {
        let mut state = ModelPickerState::new(vec![opt("a", "A", true)]);
        assert!(state.open);
        state.close();
        assert!(!state.open);
    }
}
