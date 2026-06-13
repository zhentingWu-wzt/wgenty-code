//! Interactive state for the subagent monitor panel.
//!
//! Tracks node selection, expand/collapse state, and scroll position for
//! keyboard-driven navigation within the subagent tree overlay.

use super::subagent_tree::SubagentTree;
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
    }
}
