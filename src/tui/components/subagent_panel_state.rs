//! Interactive state for the subagent monitor panel.
//!
//! Tracks node selection, expand/collapse state, and scroll position for
//! keyboard-driven navigation within the subagent tree overlay.

use super::subagent_tree::SubagentTree;
use crate::agent::progress::{SubagentEvent, SubagentStatus};
use std::collections::HashSet;

/// Interactive state for the subagent monitor panel.
#[derive(Debug, Clone, Default)]
pub struct SubagentPanelState {
    /// Index into the flattened node list (0-based).
    pub selected_index: usize,
    /// Node IDs that are currently expanded.
    pub expanded_nodes: HashSet<String>,
    /// Vertical scroll offset for the panel body.
    pub scroll_offset: u16,
    /// Detail view state for a failed/selected node (None = not in detail view).
    pub detail_view: Option<DetailViewState>,
}

impl SubagentPanelState {
    /// Build a depth-first flattened list of node IDs from the tree.
    pub fn node_list(tree: &SubagentTree) -> Vec<String> {
        let mut list = Vec::new();
        fn walk(tree: &SubagentTree, node_id: &str, list: &mut Vec<String>) {
            list.push(node_id.to_string());
            if let Some(node) = tree.nodes.get(node_id) {
                for child in &node.children {
                    walk(tree, child, list);
                }
            }
        }
        if let Some(ref root) = tree.root_id {
            walk(tree, root, &mut list);
        }
        list
    }

    /// Move selection to the previous node (wrap-around).
    pub fn move_up(&mut self, tree: &SubagentTree) {
        let list = Self::node_list(tree);
        if list.is_empty() {
            return;
        }
        if self.selected_index == 0 {
            self.selected_index = list.len() - 1;
        } else {
            self.selected_index -= 1;
        }
        self.scroll_offset = 0;
    }

    /// Move selection to the next node (wrap-around).
    pub fn move_down(&mut self, tree: &SubagentTree) {
        let list = Self::node_list(tree);
        if list.is_empty() {
            return;
        }
        self.selected_index = (self.selected_index + 1) % list.len();
        self.scroll_offset = 0;
    }

    /// Jump to first node.
    pub fn move_first(&mut self, _tree: &SubagentTree) {
        self.selected_index = 0;
        self.scroll_offset = 0;
    }

    /// Jump to last node.
    pub fn move_last(&mut self, tree: &SubagentTree) {
        let list = Self::node_list(tree);
        if !list.is_empty() {
            self.selected_index = list.len() - 1;
        }
        self.scroll_offset = 0;
    }

    /// Toggle expand/collapse for the currently selected node.
    pub fn toggle_expand(&mut self, tree: &SubagentTree) {
        let list = Self::node_list(tree);
        if let Some(node_id) = list.get(self.selected_index) {
            if self.expanded_nodes.contains(node_id) {
                self.expanded_nodes.remove(node_id);
            } else {
                self.expanded_nodes.insert(node_id.clone());
            }
        }
    }

    /// Whether a given node is currently expanded.
    pub fn is_expanded(&self, node_id: &str) -> bool {
        self.expanded_nodes.contains(node_id)
    }

    /// Get the currently selected node_id, if any.
    pub fn selected_node_id(&self, tree: &SubagentTree) -> Option<String> {
        let list = Self::node_list(tree);
        list.get(self.selected_index).cloned()
    }

    /// Reset all state (called when panel closes or new turn starts).
    pub fn reset(&mut self) {
        self.selected_index = 0;
        self.expanded_nodes.clear();
        self.scroll_offset = 0;
        self.detail_view = None;
    }

    /// Reset detail view state only.
    pub fn reset_detail(&mut self) {
        self.detail_view = None;
    }

    /// Build a DetailViewState from the currently selected node's progress data.
    /// Returns None if no node is selected or the node doesn't exist.
    /// When `status` matches Failed or Cancelled, auto-scrolls to the first Error event.
    pub fn build_detail_view(&self, tree: &SubagentTree) -> Option<DetailViewState> {
        let node_id = self.selected_node_id(tree)?;
        let node = tree.nodes.get(&node_id)?;
        let events: Vec<SubagentEvent> = node.progress.events.clone();
        let scroll_offset = if matches!(
            node.progress.status,
            SubagentStatus::Failed | SubagentStatus::Cancelled
        ) {
            events
                .iter()
                .position(|e| {
                    matches!(
                        e.event_type,
                        crate::agent::progress::SubagentEventType::Error { .. }
                    )
                })
                .unwrap_or(0)
        } else {
            0
        };
        Some(DetailViewState {
            transcript_id: node_id,
            scroll_offset,
            events,
            loading: false,
            status: Some(node.progress.status.clone()),
            total_elapsed_ms: node.progress.elapsed_ms,
            cumulative_tokens: node.progress.cumulative_tokens,
            token_budget_k: node.progress.token_budget_k,
            error_message: node
                .progress
                .metadata
                .as_ref()
                .and_then(|m| m.error.clone()),
            round: node.progress.round,
            max_rounds: node.progress.max_rounds,
        })
    }
}

/// Detail view state for a failed/selected node (None = not in detail view).
#[derive(Debug, Clone)]
pub struct DetailViewState {
    pub transcript_id: String,
    pub scroll_offset: usize,
    pub events: Vec<crate::agent::progress::SubagentEvent>,
    pub loading: bool,
    /// Overall status of the subagent.
    pub status: Option<crate::agent::progress::SubagentStatus>,
    /// Total elapsed time in ms.
    pub total_elapsed_ms: u64,
    /// Cumulative tokens used.
    pub cumulative_tokens: u64,
    /// Token budget in thousands.
    pub token_budget_k: Option<u64>,
    /// Error message (for failed/cancelled nodes).
    pub error_message: Option<String>,
    /// Round / max_rounds.
    pub round: Option<usize>,
    pub max_rounds: Option<usize>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_event() -> crate::agent::progress::SubagentEvent {
        crate::agent::progress::SubagentEvent {
            event_type: crate::agent::progress::SubagentEventType::Thought {
                text: "test thought".to_string(),
            },
            elapsed_ms: 100,
        }
    }

    #[test]
    fn test_detail_view_state_creation() {
        let events = vec![make_event()];
        let state = DetailViewState {
            transcript_id: "node1".to_string(),
            scroll_offset: 0,
            events: events.clone(),
            loading: false,
            status: None,
            total_elapsed_ms: 0,
            cumulative_tokens: 0,
            token_budget_k: None,
            error_message: None,
            round: None,
            max_rounds: None,
        };
        assert_eq!(state.transcript_id, "node1");
        assert_eq!(state.scroll_offset, 0);
        assert_eq!(state.events.len(), 1);
        assert!(!state.loading);
    }

    #[test]
    fn test_reset_detail_clears_view() {
        let mut state = SubagentPanelState::default();
        state.detail_view = Some(DetailViewState {
            transcript_id: "node1".to_string(),
            scroll_offset: 5,
            events: vec![],
            loading: false,
            status: None,
            total_elapsed_ms: 0,
            cumulative_tokens: 0,
            token_budget_k: None,
            error_message: None,
            round: None,
            max_rounds: None,
        });
        assert!(state.detail_view.is_some());
        state.reset_detail();
        assert!(state.detail_view.is_none());
    }

    #[test]
    fn test_build_detail_view_creates_state() {
        use crate::agent::progress::{SubagentProgress, SubagentStatus as Ss};

        let mut tree = SubagentTree::default();
        tree.nodes.insert(
            "n1".to_string(),
            super::super::subagent_tree::SubagentNode {
                progress: SubagentProgress {
                    node_id: "n1".to_string(),
                    parent_id: None,
                    label: "test".to_string(),
                    status: Ss::Completed,
                    round: Some(3),
                    max_rounds: Some(5),
                    current_tool: None,
                    current_params: None,
                    action_log: vec![],
                    text_snapshot: None,
                    started_at: 0,
                    elapsed_ms: 1000,
                    metadata: None,
                    progress_delta: None,
                    token_budget_k: Some(10),
                    cumulative_tokens: 500,
                    error_details: None,
                    events: vec![],
                },
                children: vec![],
            },
        );
        tree.root_id = Some("n1".to_string());

        // Default state has selected_index=0, should find "n1"
        let state = SubagentPanelState::default();
        let detail = state
            .build_detail_view(&tree)
            .expect("should build detail for selected node");
        assert_eq!(detail.transcript_id, "n1");
        assert!(detail.events.is_empty());
        assert_eq!(detail.status, Some(Ss::Completed));
        assert_eq!(detail.total_elapsed_ms, 1000);
        assert_eq!(detail.cumulative_tokens, 500);
        assert_eq!(detail.token_budget_k, Some(10));
        assert_eq!(detail.round, Some(3));
        assert_eq!(detail.max_rounds, Some(5));
    }

    #[test]
    fn test_reset_clears_detail_view() {
        let mut state = SubagentPanelState::default();
        state.detail_view = Some(DetailViewState {
            transcript_id: "node1".to_string(),
            scroll_offset: 0,
            events: vec![],
            loading: true,
            status: None,
            total_elapsed_ms: 0,
            cumulative_tokens: 0,
            token_budget_k: None,
            error_message: None,
            round: None,
            max_rounds: None,
        });
        state.reset();
        assert!(state.detail_view.is_none());
        assert_eq!(state.selected_index, 0);
        assert!(state.expanded_nodes.is_empty());
    }

    #[test]
    fn test_detail_view_scroll_offset_saturating() {
        let mut detail = DetailViewState {
            transcript_id: "node1".to_string(),
            scroll_offset: 0,
            events: vec![make_event(), make_event(), make_event()],
            loading: false,
            status: None,
            total_elapsed_ms: 0,
            cumulative_tokens: 0,
            token_budget_k: None,
            error_message: None,
            round: None,
            max_rounds: None,
        };
        // saturating_sub should not go below 0
        detail.scroll_offset = detail.scroll_offset.saturating_sub(1);
        assert_eq!(detail.scroll_offset, 0);

        // saturating_add should increase
        detail.scroll_offset = detail.scroll_offset.saturating_add(1);
        assert_eq!(detail.scroll_offset, 1);

        // Jump to last event
        detail.scroll_offset = detail.events.len().saturating_sub(1);
        assert_eq!(detail.scroll_offset, 2);
    }
}
