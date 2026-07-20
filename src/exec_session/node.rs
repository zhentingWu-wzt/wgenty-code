//! Node-level data structures for the ExecutionSession outer layer.
//!
//! This module defines the node contract, status machine, and node record
//! types that persist to `session.json`'s `node_states` field. The node is
//! an aggregation of turns toward a verifiable goal.

use serde::{Deserialize, Serialize};

/// Type alias for node identifiers (e.g. "n1", "n2").
pub type NodeId = String;

/// Agent-declared node contract. The runtime trusts the declaration and
/// executes verify commands itself (never trusts agent-asserted results).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NodeContract {
    /// Human-readable goal description.
    pub goal: String,
    /// Verify commands the runtime executes (via guardian + sandbox).
    pub verify_commands: Vec<String>,
    /// Out-of-bounds detection boundary. Empty = no boundary check.
    pub expected_files: Vec<String>,
}

/// Node state machine states.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NodeStatus {
    /// Created but not started (transient, immediately transitions to Running).
    Pending,
    /// Agent is working within this node.
    Running,
    /// verify_node is executing.
    Verifying,
    /// Verify passed - safe checkpoint.
    Verified,
    /// Verify failed - agent can self-correct and retry.
    Failed,
}

/// One node record in the session's node chain.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Node {
    /// Node identifier (e.g. "n1", "n2").
    pub id: NodeId,
    /// The agent-declared contract.
    pub contract: NodeContract,
    /// Current state machine status.
    pub status: NodeStatus,
    /// The turn_id when this node began; verify/rollback range start.
    pub start_turn_id: String,
    /// Number of verify failures for this node.
    pub retry_count: u32,
    /// Path to the verify log file for this node.
    pub verify_log_path: String,
    /// RFC 3339 creation timestamp.
    pub created_at: String,
}

/// The node chain persisted in session.json's `node_states` field.
/// Linear chain, no nesting.
pub type NodeStates = Vec<Node>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_node_contract_serialization() {
        let contract = NodeContract {
            goal: "add memory clear command".to_string(),
            verify_commands: vec!["cargo test".to_string(), "cargo clippy".to_string()],
            expected_files: vec!["src/cli.rs".to_string(), "src/memory/list.rs".to_string()],
        };
        let json = serde_json::to_string(&contract).expect("serialize");
        let deserialized: NodeContract = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(contract, deserialized);
    }

    #[test]
    fn test_node_contract_empty_expected_files() {
        let contract = NodeContract {
            goal: "explore codebase".to_string(),
            verify_commands: vec!["echo ok".to_string()],
            expected_files: vec![],
        };
        let json = serde_json::to_string(&contract).expect("serialize");
        let deserialized: NodeContract = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(contract, deserialized);
        assert!(deserialized.expected_files.is_empty());
    }

    #[test]
    fn test_node_status_serialization_snake_case() {
        let cases = vec![
            (NodeStatus::Pending, "pending"),
            (NodeStatus::Running, "running"),
            (NodeStatus::Verifying, "verifying"),
            (NodeStatus::Verified, "verified"),
            (NodeStatus::Failed, "failed"),
        ];
        for (status, expected_json) in cases {
            let json = serde_json::to_string(&status).expect("serialize");
            assert_eq!(
                json,
                format!("\"{}\"", expected_json),
                "NodeStatus::{:?} should serialize as \"{}\"",
                status,
                expected_json
            );
            let deserialized: NodeStatus = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(status, deserialized);
        }
    }

    #[test]
    fn test_node_serialization_round_trip() {
        let node = Node {
            id: "n1".to_string(),
            contract: NodeContract {
                goal: "add memory clear command".to_string(),
                verify_commands: vec!["cargo test".to_string()],
                expected_files: vec!["src/cli.rs".to_string()],
            },
            status: NodeStatus::Running,
            start_turn_id: "turn-0".to_string(),
            retry_count: 0,
            verify_log_path: "snapshots/es-a3f2c1/verify_log_n1.json".to_string(),
            created_at: "2026-07-20T10:00:00Z".to_string(),
        };
        let json = serde_json::to_string(&node).expect("serialize");
        let deserialized: Node = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(node, deserialized);
    }

    #[test]
    fn test_node_states_serialization() {
        let nodes = vec![
            Node {
                id: "n1".to_string(),
                contract: NodeContract {
                    goal: "task 1".to_string(),
                    verify_commands: vec!["echo ok".to_string()],
                    expected_files: vec![],
                },
                status: NodeStatus::Verified,
                start_turn_id: "turn-0".to_string(),
                retry_count: 0,
                verify_log_path: "log1.json".to_string(),
                created_at: "2026-07-20T10:00:00Z".to_string(),
            },
            Node {
                id: "n2".to_string(),
                contract: NodeContract {
                    goal: "task 2".to_string(),
                    verify_commands: vec!["echo ok".to_string()],
                    expected_files: vec![],
                },
                status: NodeStatus::Running,
                start_turn_id: "turn-3".to_string(),
                retry_count: 0,
                verify_log_path: "log2.json".to_string(),
                created_at: "2026-07-20T11:00:00Z".to_string(),
            },
        ];
        let json = serde_json::to_string(&nodes).expect("serialize");
        let deserialized: NodeStates = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(nodes, deserialized);
        assert_eq!(deserialized.len(), 2);
    }
}
