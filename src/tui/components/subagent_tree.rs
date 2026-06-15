//! SubagentTree — in-memory tree state for subagent execution progress.

use crate::agent::progress::{SubagentProgress, SubagentStatus};
use std::collections::HashMap;

#[derive(Debug, Clone, Default)]
pub struct SubagentTree {
    pub root_id: Option<String>,
    pub nodes: HashMap<String, SubagentNode>,
}

#[derive(Debug, Clone)]
pub struct SubagentNode {
    pub progress: SubagentProgress,
    pub children: Vec<String>,
}

impl SubagentTree {
    pub fn upsert(&mut self, progress: SubagentProgress) {
        let node_id = progress.node_id.clone();
        let parent_id = progress.parent_id.clone();

        if parent_id.is_none() && self.root_id.is_none() {
            self.root_id = Some(node_id.clone());
        }

        if let Some(ref pid) = parent_id {
            if let Some(parent) = self.nodes.get_mut(pid) {
                if !parent.children.contains(&node_id) {
                    parent.children.push(node_id.clone());
                }
            }
        }

        match self.nodes.get_mut(&node_id) {
            Some(existing) => {
                existing.progress = progress;
            }
            None => {
                self.nodes.insert(
                    node_id,
                    SubagentNode {
                        progress,
                        children: Vec::new(),
                    },
                );
            }
        }
    }

    pub fn count_by_status(&self, status: SubagentStatus) -> usize {
        self.nodes
            .values()
            .filter(|n| n.progress.status == status)
            .count()
    }

    /// Number of currently active (Running + Pending) subagent nodes.
    pub fn active_count(&self) -> usize {
        self.count_by_status(SubagentStatus::Running)
            + self.count_by_status(SubagentStatus::Pending)
    }

    /// Number of successfully completed subagent nodes.
    pub fn completed_count(&self) -> usize {
        self.count_by_status(SubagentStatus::Completed)
    }

    /// Number of failed or cancelled subagent nodes.
    pub fn failed_count(&self) -> usize {
        self.count_by_status(SubagentStatus::Failed)
            + self.count_by_status(SubagentStatus::Cancelled)
    }

    /// Total number of nodes in the tree.
    pub fn total_count(&self) -> usize {
        self.nodes.len()
    }

    /// Whether the tree has any nodes at all.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn is_complete(&self) -> bool {
        self.nodes.values().all(|n| {
            matches!(
                n.progress.status,
                SubagentStatus::Completed | SubagentStatus::Failed | SubagentStatus::Cancelled
            )
        })
    }

    pub fn clear(&mut self) {
        self.root_id = None;
        self.nodes.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_progress(
        node_id: &str,
        parent_id: Option<&str>,
        status: SubagentStatus,
    ) -> SubagentProgress {
        SubagentProgress {
            node_id: node_id.to_string(),
            parent_id: parent_id.map(String::from),
            label: format!("Node {}", node_id),
            status,
            round: None,
            max_rounds: None,
            current_tool: None,
            current_params: None,
            action_log: Vec::new(),
            text_snapshot: None,
            started_at: 0,
            elapsed_ms: 0,
            metadata: None,
            progress_delta: None,
            token_budget_k: None,
            cumulative_tokens: 0,
            error_details: None,
            events: Vec::new(),
        }
    }

    #[test]
    fn test_upsert_creates_tree() {
        let mut tree = SubagentTree::default();
        tree.upsert(make_progress("root", None, SubagentStatus::Running));
        assert_eq!(tree.root_id.as_deref(), Some("root"));
        assert_eq!(tree.nodes.len(), 1);
        tree.upsert(make_progress(
            "child1",
            Some("root"),
            SubagentStatus::Running,
        ));
        assert_eq!(tree.nodes.len(), 2);
        assert_eq!(tree.nodes["root"].children, vec!["child1"]);
        tree.upsert(make_progress(
            "child1",
            Some("root"),
            SubagentStatus::Completed,
        ));
        assert_eq!(
            tree.nodes["child1"].progress.status,
            SubagentStatus::Completed
        );
    }

    #[test]
    fn test_is_complete() {
        let mut tree = SubagentTree::default();
        tree.upsert(make_progress("root", None, SubagentStatus::Completed));
        tree.upsert(make_progress("a", Some("root"), SubagentStatus::Completed));
        assert!(tree.is_complete());
        tree.upsert(make_progress("b", Some("root"), SubagentStatus::Running));
        assert!(!tree.is_complete());
    }

    #[test]
    fn test_count_by_status() {
        let mut tree = SubagentTree::default();
        tree.upsert(make_progress("root", None, SubagentStatus::Completed));
        tree.upsert(make_progress("a", Some("root"), SubagentStatus::Completed));
        tree.upsert(make_progress("b", Some("root"), SubagentStatus::Running));
        tree.upsert(make_progress("c", Some("root"), SubagentStatus::Pending));
        assert_eq!(tree.count_by_status(SubagentStatus::Completed), 2);
        assert_eq!(tree.count_by_status(SubagentStatus::Running), 1);
        assert_eq!(tree.count_by_status(SubagentStatus::Pending), 1);
    }
}
