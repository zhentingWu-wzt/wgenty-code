//! SubagentTree — in-memory tree state for subagent execution progress.

use crate::agent::progress::{SubagentProgress, SubagentStatus};
use crate::daemon::models::LocalAgentViewResponse;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Default)]
pub struct SubagentTree {
    pub root_id: Option<String>,
    pub nodes: HashMap<String, SubagentNode>,
    /// Lightweight navigation metadata for the current scoped local view.
    /// Conversation payloads live only in `nodes`.
    local_view: Option<ScopedLocalViewMetadata>,
}

#[derive(Debug, Clone)]
struct ScopedLocalViewMetadata {
    self_agent_id: String,
    children: Vec<ScopedChildNavigation>,
}

#[derive(Debug, Clone)]
struct ScopedChildNavigation {
    agent_id: String,
    navigation_capability: String,
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

    /// Replaces the tree with the given scoped local view. Previous layer
    /// nodes are removed; only self + direct children populate the tree.
    pub fn replace_local(&mut self, view: LocalAgentViewResponse) {
        let LocalAgentViewResponse {
            self_view,
            children,
        } = view;
        self.nodes.clear();
        self.root_id = Some(self_view.agent_id.clone());
        let child_ids: Vec<String> = children.iter().map(|c| c.agent_id.clone()).collect();
        let local_view = ScopedLocalViewMetadata {
            self_agent_id: self_view.agent_id.clone(),
            children: children
                .iter()
                .map(|child| ScopedChildNavigation {
                    agent_id: child.agent_id.clone(),
                    navigation_capability: child.navigation_capability.clone(),
                })
                .collect(),
        };
        self.nodes.insert(
            self_view.agent_id.clone(),
            SubagentNode {
                progress: SubagentProgress {
                    node_id: self_view.agent_id,
                    parent_id: None,
                    label: self_view.label,
                    status: self_view.status.into(),
                    round: self_view.round,
                    max_rounds: self_view.max_rounds,
                    current_tool: None,
                    current_params: None,
                    action_log: Vec::new(),
                    text_snapshot: self_view.text_snapshot,
                    started_at: self_view.started_at,
                    elapsed_ms: self_view.elapsed_ms,
                    metadata: None,
                    progress_delta: None,
                    token_budget_k: None,
                    cumulative_tokens: self_view.cumulative_tokens,
                    error_details: None,
                    events: Vec::new(),
                    messages: self_view.messages,
                },
                children: child_ids,
            },
        );
        for child in children {
            self.nodes.insert(
                child.agent_id.clone(),
                SubagentNode {
                    progress: SubagentProgress {
                        node_id: child.agent_id,
                        parent_id: Some(local_view.self_agent_id.clone()),
                        label: child.label,
                        status: child.status.into(),
                        text_snapshot: child.text_snapshot,
                        cumulative_tokens: child.cumulative_tokens,
                        messages: child.messages,
                        round: child.round,
                        max_rounds: child.max_rounds,
                        current_tool: None,
                        current_params: None,
                        action_log: Vec::new(),
                        started_at: child.started_at,
                        elapsed_ms: child.elapsed_ms,
                        metadata: None,
                        progress_delta: None,
                        token_budget_k: None,
                        error_details: None,
                        events: Vec::new(),
                    },
                    children: Vec::new(),
                },
            );
        }
        self.local_view = Some(local_view);
    }

    /// Returns sorted selectable node ids (self + direct children).
    pub fn selectable_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self
            .nodes
            .values()
            .map(|n| n.progress.node_id.clone())
            .collect();
        ids.sort();
        ids
    }

    pub fn contains(&self, node_id: &str) -> bool {
        self.nodes.contains_key(node_id)
    }

    /// Returns the opaque navigation capability for a direct child of the
    /// currently loaded local view, if any. Capabilities are viewer-bound and
    /// issued by the daemon; raw agent ids confer no authority.
    pub fn capability_for_child(&self, child_id: &str) -> Option<String> {
        let view = self.local_view.as_ref()?;
        view.children
            .iter()
            .find(|c| c.agent_id == child_id)
            .map(|c| c.navigation_capability.clone())
    }

    pub fn count_by_status(&self, status: SubagentStatus) -> usize {
        self.real_node_list()
            .iter()
            .filter_map(|id| self.nodes.get(id))
            .filter(|n| n.progress.status == status)
            .count()
    }

    /// Number of nonterminal subagent nodes.
    pub fn active_count(&self) -> usize {
        self.real_node_list()
            .iter()
            .filter_map(|id| self.nodes.get(id))
            .filter(|node| !node.progress.status.is_terminal())
            .count()
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
    ///
    /// Supports a forest of independent roots: walks `root_id` (the first
    /// top-level node) first, then any other `parent_id: None` nodes not yet
    /// visited. This ensures every top-level subagent (e.g., multiple `task`
    /// subagents after D7 removed the wrapper root) is reachable — without it,
    /// subsequent top-level subagents would be orphans and never appear in the
    /// status bar / selector. A visited set prevents duplicates (test fixtures
    /// and any re-linked node are only walked once).
    pub fn node_list(&self) -> Vec<String> {
        let mut list = Vec::new();
        let mut visited: HashSet<String> = HashSet::new();
        fn walk(
            tree: &SubagentTree,
            node_id: &str,
            list: &mut Vec<String>,
            visited: &mut HashSet<String>,
        ) {
            if !visited.insert(node_id.to_string()) {
                return;
            }
            list.push(node_id.to_string());
            if let Some(node) = tree.nodes.get(node_id) {
                for child in &node.children {
                    walk(tree, child, list, visited);
                }
            }
        }
        // Walk the primary root first (back-compat: preserves DFS start + order).
        if let Some(ref root) = self.root_id {
            walk(self, root, &mut list, &mut visited);
        }
        // Forest: walk any other top-level nodes (parent_id None) not yet visited.
        // Deterministic order by started_at, then node_id.
        let mut extra: Vec<String> = self
            .nodes
            .iter()
            .filter(|(id, n)| n.progress.parent_id.is_none() && !visited.contains(id.as_str()))
            .map(|(id, _)| id.clone())
            .collect();
        extra.sort_by(|a, b| {
            let sa = self
                .nodes
                .get(a)
                .map(|n| n.progress.started_at)
                .unwrap_or(0);
            let sb = self
                .nodes
                .get(b)
                .map(|n| n.progress.started_at)
                .unwrap_or(0);
            sa.cmp(&sb).then_with(|| a.cmp(b))
        });
        for root in &extra {
            walk(self, root, &mut list, &mut visited);
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
            .all(|node| node.progress.status.is_terminal())
    }

    pub fn clear(&mut self) {
        self.root_id = None;
        self.nodes.clear();
    }

    /// Clear the tree only when all subagents are terminal.
    ///
    /// Background subagents (task tool `background` mode) outlive the main
    /// turn — they are spawned via `tokio::spawn` and the main turn returns
    /// immediately. When the next turn starts, unconditionally clearing the
    /// tree would wipe these still-running background subagents, hiding the
    /// status bar and blocking entry to their focus view. This method
    /// preserves active subagents across the turn boundary. Returns `true`
    /// if the tree was cleared.
    pub fn clear_if_idle(&mut self) -> bool {
        if self.active_count() == 0 {
            self.clear();
            true
        } else {
            false
        }
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
        tree.upsert(make_progress(
            "delegate-root",
            None,
            SubagentStatus::Running,
        ));
        tree.upsert(make_progress(
            "sub1",
            Some("delegate-root"),
            SubagentStatus::Running,
        ));
        tree.upsert(make_progress(
            "sub2",
            Some("delegate-root"),
            SubagentStatus::Completed,
        ));

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
        tree.upsert(make_progress(
            "delegate-root",
            None,
            SubagentStatus::Running,
        ));
        tree.upsert(make_progress(
            "sub1",
            Some("delegate-root"),
            SubagentStatus::Running,
        ));
        tree.upsert(make_progress(
            "sub2",
            Some("delegate-root"),
            SubagentStatus::Completed,
        ));
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
        tree.upsert(make_progress(
            "delegate-root",
            None,
            SubagentStatus::Running,
        ));
        tree.upsert(make_progress(
            "sub1",
            Some("delegate-root"),
            SubagentStatus::Completed,
        ));
        tree.upsert(make_progress(
            "sub2",
            Some("delegate-root"),
            SubagentStatus::Completed,
        ));
        // Without filtering, wrapper's Running would make is_complete false.
        // With filtering, all real nodes completed → is_complete true.
        assert!(tree.is_complete());
    }

    #[test]
    fn test_node_list_multiple_top_level_roots() {
        let mut tree = SubagentTree::default();
        // Three independent top-level subagents (parent_id None) — e.g., multiple
        // `task` subagents after D7 removed the wrapper root.
        tree.upsert(make_progress("sub1", None, SubagentStatus::Running));
        tree.upsert(make_progress("sub2", None, SubagentStatus::Running));
        tree.upsert(make_progress("sub3", None, SubagentStatus::Pending));
        // All three must appear in node_list (forest), not just the first.
        let list = tree.node_list();
        assert!(list.contains(&"sub1".to_string()));
        assert!(list.contains(&"sub2".to_string()));
        assert!(list.contains(&"sub3".to_string()));
        assert_eq!(list.len(), 3, "all top-level roots must be reachable");
        // active_count covers all roots (none are grouping nodes — all leaves)
        assert_eq!(tree.active_count(), 3);
    }

    #[test]
    fn test_node_list_delegate_plus_independent_task() {
        let mut tree = SubagentTree::default();
        // delegate wrapper (first root) + its sub-tasks
        tree.upsert(make_progress("delegate", None, SubagentStatus::Running));
        tree.upsert(make_progress(
            "dt1",
            Some("delegate"),
            SubagentStatus::Running,
        ));
        tree.upsert(make_progress(
            "dt2",
            Some("delegate"),
            SubagentStatus::Running,
        ));
        // independent task subagent (separate top-level root, parent_id None)
        tree.upsert(make_progress("task-sub", None, SubagentStatus::Running));
        // node_list must reach ALL roots: delegate + dt1 + dt2 + task-sub
        let list = tree.node_list();
        assert!(list.contains(&"delegate".to_string()));
        assert!(list.contains(&"dt1".to_string()));
        assert!(list.contains(&"dt2".to_string()));
        assert!(list.contains(&"task-sub".to_string()));
        assert_eq!(list.len(), 4, "independent task-sub must not be orphaned");
        // active_count excludes the delegate grouping node: dt1 + dt2 + task-sub
        assert_eq!(tree.active_count(), 3);
    }

    #[test]
    fn replace_local_preserves_child_label_for_selector() {
        let mut tree = SubagentTree::default();
        tree.replace_local(crate::daemon::models::LocalAgentViewResponse {
            self_view: crate::daemon::models::SelfAgentResponse {
                agent_id: "root".to_string(),
                status: crate::agent::AgentLifecycleStatus::Running,
                label: String::new(),
                text_snapshot: None,
                cumulative_tokens: 0,
                messages: Vec::new(),
                started_at: 0,
                elapsed_ms: 0,
                round: None,
                max_rounds: None,
            },
            children: vec![crate::daemon::models::DirectChildResponse {
                agent_id: "child".to_string(),
                status: crate::agent::AgentLifecycleStatus::Running,
                label: "inspect selector labels".to_string(),
                summary: None,
                navigation_capability: "capability".to_string(),
                text_snapshot: None,
                cumulative_tokens: 0,
                messages: Vec::new(),
                started_at: 0,
                elapsed_ms: 0,
                round: None,
                max_rounds: None,
            }],
        });

        assert_eq!(
            tree.nodes["child"].progress.label,
            "inspect selector labels"
        );
    }

    #[test]
    fn replace_local_populates_self_conversation_and_metadata() {
        let mut tree = SubagentTree::default();
        tree.replace_local(crate::daemon::models::LocalAgentViewResponse {
            self_view: crate::daemon::models::SelfAgentResponse {
                agent_id: "child".to_string(),
                status: crate::agent::AgentLifecycleStatus::Running,
                label: "Child task".to_string(),
                text_snapshot: Some("snapshot-child".to_string()),
                cumulative_tokens: 42,
                messages: vec![crate::api::ChatMessage::assistant("child answer")],
                started_at: 0,
                elapsed_ms: 0,
                round: None,
                max_rounds: None,
            },
            children: Vec::new(),
        });

        let progress = &tree.nodes["child"].progress;
        assert_eq!(progress.label, "Child task");
        assert_eq!(progress.text_snapshot.as_deref(), Some("snapshot-child"));
        assert_eq!(progress.cumulative_tokens, 42);
        assert_eq!(progress.messages.len(), 1);
        assert_eq!(progress.messages[0].role, "assistant");
        assert_eq!(
            progress.messages[0].content.as_deref(),
            Some("child answer")
        );
    }

    #[test]
    fn capability_for_child_returns_direct_child_capability_only() {
        // Only direct children of the loaded local view expose a navigation
        // capability; the self node and unknown ids return None.
        let mut tree = SubagentTree::default();
        tree.replace_local(crate::daemon::models::LocalAgentViewResponse {
            self_view: crate::daemon::models::SelfAgentResponse {
                agent_id: "root".to_string(),
                status: crate::agent::AgentLifecycleStatus::Running,
                label: String::new(),
                text_snapshot: None,
                cumulative_tokens: 0,
                messages: Vec::new(),
                started_at: 0,
                elapsed_ms: 0,
                round: None,
                max_rounds: None,
            },
            children: vec![crate::daemon::models::DirectChildResponse {
                agent_id: "child".to_string(),
                status: crate::agent::AgentLifecycleStatus::Running,
                label: "child".to_string(),
                summary: None,
                navigation_capability: "cap-child".to_string(),
                text_snapshot: None,
                cumulative_tokens: 0,
                messages: Vec::new(),
                started_at: 0,
                elapsed_ms: 0,
                round: None,
                max_rounds: None,
            }],
        });

        assert_eq!(
            tree.capability_for_child("child").as_deref(),
            Some("cap-child")
        );
        // Self node has no capability.
        assert!(tree.capability_for_child("root").is_none());
        // Unknown / hidden id has no capability.
        assert!(tree.capability_for_child("grandchild").is_none());
        // No loaded view -> None.
        let empty = SubagentTree::default();
        assert!(empty.capability_for_child("anything").is_none());
    }

    #[test]
    fn replace_local_caches_navigation_metadata_without_response_payload() {
        let mut tree = SubagentTree::default();
        tree.replace_local(crate::daemon::models::LocalAgentViewResponse {
            self_view: crate::daemon::models::SelfAgentResponse {
                agent_id: "root".to_string(),
                status: crate::agent::AgentLifecycleStatus::Running,
                label: "Root".to_string(),
                text_snapshot: Some("root snapshot".to_string()),
                cumulative_tokens: 7,
                messages: vec![crate::api::ChatMessage::assistant("root answer")],
                started_at: 0,
                elapsed_ms: 0,
                round: None,
                max_rounds: None,
            },
            children: vec![crate::daemon::models::DirectChildResponse {
                agent_id: "child".to_string(),
                status: crate::agent::AgentLifecycleStatus::Running,
                label: "Child".to_string(),
                summary: None,
                navigation_capability: "cap-child".to_string(),
                text_snapshot: Some("child snapshot".to_string()),
                cumulative_tokens: 11,
                messages: vec![crate::api::ChatMessage::assistant("child answer")],
                started_at: 0,
                elapsed_ms: 0,
                round: None,
                max_rounds: None,
            }],
        });

        let cached = tree.local_view.as_ref().expect("local view metadata");
        assert_eq!(cached.self_agent_id, "root");
        assert_eq!(cached.children.len(), 1);
        assert_eq!(cached.children[0].agent_id, "child");
        assert_eq!(cached.children[0].navigation_capability, "cap-child");
        assert_eq!(tree.nodes["root"].progress.messages.len(), 1);
        assert_eq!(tree.nodes["child"].progress.messages.len(), 1);
    }

    #[test]
    fn test_clear_if_idle_preserves_active_subagent() {
        // A background subagent is still Running when the next turn starts.
        let mut tree = SubagentTree::default();
        tree.upsert(make_progress("bg", None, SubagentStatus::Running));
        tree.root_id = Some("bg".to_string());
        assert!(
            !tree.clear_if_idle(),
            "must not clear while a subagent is active"
        );
        assert!(
            tree.nodes.contains_key("bg"),
            "background subagent must be preserved"
        );
        assert_eq!(tree.active_count(), 1);
    }

    #[test]
    fn test_active_count_and_clear_if_idle_cover_every_nonterminal_status() {
        for status in [
            SubagentStatus::Pending,
            SubagentStatus::Running,
            SubagentStatus::WaitingForChildren,
            SubagentStatus::Finalizing,
            SubagentStatus::Cancelling,
        ] {
            let mut tree = SubagentTree::default();
            tree.upsert(make_progress("active", None, status.clone()));

            assert_eq!(tree.active_count(), 1, "{status:?} must count as active");
            assert!(
                !tree.clear_if_idle(),
                "{status:?} must prevent the tree from being cleared"
            );
            assert!(tree.nodes.contains_key("active"));
        }
    }

    #[test]
    fn test_clear_if_idle_clears_when_only_terminal_remain() {
        // Previous turn's foreground subagents are Completed; no active remain.
        let mut tree = SubagentTree::default();
        tree.upsert(make_progress("done", None, SubagentStatus::Completed));
        tree.upsert(make_progress("failed", None, SubagentStatus::Failed));
        tree.root_id = Some("done".to_string());
        assert!(
            tree.clear_if_idle(),
            "must clear when no active subagents remain"
        );
        assert!(tree.nodes.is_empty(), "terminal nodes must be cleared");
        assert!(tree.root_id.is_none());
    }

    #[test]
    fn test_clear_if_idle_clears_each_terminal_status() {
        for status in [
            SubagentStatus::Completed,
            SubagentStatus::Failed,
            SubagentStatus::Cancelled,
        ] {
            let mut tree = SubagentTree::default();
            tree.upsert(make_progress("terminal", None, status.clone()));

            assert!(tree.clear_if_idle(), "{status:?} must allow clearing");
            assert!(tree.is_empty());
        }
    }

    #[test]
    fn test_clear_if_idle_clears_empty_tree() {
        let mut tree = SubagentTree::default();
        assert!(tree.clear_if_idle(), "empty tree should report cleared");
        assert!(tree.nodes.is_empty());
    }

    #[test]
    fn test_clear_if_idle_preserves_running_amid_terminal() {
        // Mixed: one Completed (previous turn) + one Running (background).
        // clear_if_idle must preserve the Running one.
        let mut tree = SubagentTree::default();
        tree.upsert(make_progress("done", None, SubagentStatus::Completed));
        tree.upsert(make_progress("bg", None, SubagentStatus::Running));
        tree.root_id = Some("done".to_string());
        assert!(
            !tree.clear_if_idle(),
            "must not clear while bg subagent is active"
        );
        assert!(
            tree.nodes.contains_key("bg"),
            "background subagent preserved"
        );
        assert!(
            tree.nodes.contains_key("done"),
            "completed node also retained (no partial clear)"
        );
    }
}
