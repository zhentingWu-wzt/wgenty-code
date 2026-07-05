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
        self.real_node_list()
            .iter()
            .filter_map(|id| self.nodes.get(id))
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

    /// Total cumulative tokens across all nodes.
    pub fn total_tokens(&self) -> u64 {
        self.nodes
            .values()
            .map(|n| n.progress.cumulative_tokens)
            .sum()
    }

    /// Current token budget (highest among running nodes).
    pub fn active_budget_k(&self) -> Option<u64> {
        self.nodes
            .values()
            .filter(|n| n.progress.status == SubagentStatus::Running)
            .filter_map(|n| n.progress.token_budget_k)
            .max()
    }

    /// Total number of real nodes in the tree (grouping nodes excluded).
    pub fn total_count(&self) -> usize {
        self.real_node_list().len()
    }

    /// Whether the tree has any nodes at all.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Depth-first flattened list of all node IDs in the tree.
    pub fn node_list(&self) -> Vec<String> {
        let mut list = Vec::new();
        fn walk(tree: &SubagentTree, node_id: &str, list: &mut Vec<String>) {
            list.push(node_id.to_string());
            if let Some(node) = tree.nodes.get(node_id) {
                for child in &node.children {
                    walk(tree, child, list);
                }
            }
        }
        if let Some(ref root) = self.root_id {
            walk(self, root, &mut list);
        }
        list
    }

    /// Whether a node is a grouping/wrapper node with no execution info of its own.
    /// Grouping nodes have children but no events or messages (e.g., a `delegate`
    /// 1:N wrapper that is never updated). Real subagents — even Pending leaves
    /// with no events yet — are never grouping nodes (they have no children).
    pub fn is_grouping_node(&self, node_id: &str) -> bool {
        match self.nodes.get(node_id) {
            Some(n) => {
                !n.children.is_empty()
                    && n.progress.events.is_empty()
                    && n.progress.messages.is_empty()
            }
            None => false,
        }
    }

    /// Depth-first list of all REAL node IDs (grouping nodes excluded).
    pub fn real_node_list(&self) -> Vec<String> {
        self.node_list()
            .into_iter()
            .filter(|id| !self.is_grouping_node(id))
            .collect()
    }

    pub fn is_complete(&self) -> bool {
        self.real_node_list()
            .iter()
            .filter_map(|id| self.nodes.get(id))
            .all(|n| {
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
            messages: Vec::new(),
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
        // root has children + no events → grouping node, excluded from counts (D9)
        tree.upsert(make_progress("root", None, SubagentStatus::Completed));
        tree.upsert(make_progress("a", Some("root"), SubagentStatus::Completed));
        tree.upsert(make_progress("b", Some("root"), SubagentStatus::Running));
        tree.upsert(make_progress("c", Some("root"), SubagentStatus::Pending));
        // root (Completed) excluded as a grouping node; only real children counted
        assert_eq!(tree.count_by_status(SubagentStatus::Completed), 1); // a
        assert_eq!(tree.count_by_status(SubagentStatus::Running), 1); // b
        assert_eq!(tree.count_by_status(SubagentStatus::Pending), 1); // c
        // total_count also excludes the grouping root
        assert_eq!(tree.total_count(), 3);
    }

    #[test]
    fn test_node_list_flat() {
        let mut tree = SubagentTree::default();
        tree.upsert(make_progress("root", None, SubagentStatus::Running));
        let list = tree.node_list();
        assert_eq!(list, vec!["root"]);
    }

    #[test]
    fn test_node_list_tree() {
        let mut tree = SubagentTree::default();
        tree.upsert(make_progress("root", None, SubagentStatus::Running));
        tree.upsert(make_progress("a", Some("root"), SubagentStatus::Running));
        tree.upsert(make_progress("b", Some("root"), SubagentStatus::Running));
        // grandchild under "a"
        tree.upsert(make_progress("a1", Some("a"), SubagentStatus::Running));
        let list = tree.node_list();
        // DFS: root → a → a1 → b
        assert_eq!(list, vec!["root", "a", "a1", "b"]);
    }

    #[test]
    fn test_node_list_empty() {
        let tree = SubagentTree::default();
        assert!(tree.node_list().is_empty());
    }

    #[test]
    fn test_is_grouping_node_and_real_node_list() {
        let mut tree = SubagentTree::default();
        // delegate wrapper: has children, no events/messages
        tree.upsert(make_progress("delegate-root", None, SubagentStatus::Running));
        tree.upsert(make_progress("sub1", Some("delegate-root"), SubagentStatus::Running));
        tree.upsert(make_progress("sub2", Some("delegate-root"), SubagentStatus::Completed));

        // wrapper is a grouping node (has children, no events/messages)
        assert!(tree.is_grouping_node("delegate-root"));
        // real subagents are not grouping nodes (no children)
        assert!(!tree.is_grouping_node("sub1"));
        assert!(!tree.is_grouping_node("sub2"));

        // real_node_list excludes grouping nodes
        let real = tree.real_node_list();
        assert_eq!(real.len(), 2);
        assert!(real.contains(&"sub1".to_string()));
        assert!(real.contains(&"sub2".to_string()));
        assert!(!real.contains(&"delegate-root".to_string()));

        // a lone root with no children is NOT a grouping node (real subagent)
        let mut tree2 = SubagentTree::default();
        tree2.upsert(make_progress("solo-root", None, SubagentStatus::Running));
        assert!(!tree2.is_grouping_node("solo-root"));
        assert_eq!(tree2.real_node_list(), vec!["solo-root".to_string()]);
    }

    #[test]
    fn test_active_count_excludes_grouping_nodes() {
        let mut tree = SubagentTree::default();
        // wrapper stuck in Running (never updated) + 2 real children
        tree.upsert(make_progress("delegate-root", None, SubagentStatus::Running));
        tree.upsert(make_progress("sub1", Some("delegate-root"), SubagentStatus::Running));
        tree.upsert(make_progress("sub2", Some("delegate-root"), SubagentStatus::Completed));
        // Without filtering, wrapper would inflate active_count to 2 (wrapper + sub1).
        // With filtering, active_count = 1 (only sub1; sub2 completed).
        assert_eq!(tree.active_count(), 1);
        // total_count excludes wrapper: 2 real nodes, not 3
        assert_eq!(tree.total_count(), 2);
        // count_by_status also excludes wrapper
        assert_eq!(tree.count_by_status(SubagentStatus::Running), 1); // sub1 only
        assert_eq!(tree.count_by_status(SubagentStatus::Completed), 1); // sub2
    }

    #[test]
    fn test_is_complete_excludes_grouping_nodes() {
        let mut tree = SubagentTree::default();
        // wrapper stuck Running + all real children completed
        tree.upsert(make_progress("delegate-root", None, SubagentStatus::Running));
        tree.upsert(make_progress("sub1", Some("delegate-root"), SubagentStatus::Completed));
        tree.upsert(make_progress("sub2", Some("delegate-root"), SubagentStatus::Completed));
        // Without filtering, wrapper's Running would make is_complete false.
        // With filtering, all real nodes completed → is_complete true.
        assert!(tree.is_complete());
    }
}
