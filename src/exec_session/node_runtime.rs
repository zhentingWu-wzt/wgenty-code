//! NodeRuntime: coordinates the node-level state machine for the
//! ExecutionSession outer layer.
//!
//! A node is an aggregation of turns toward a verifiable goal. The runtime
//! manages node creation ([`begin_node`]), verification
//! ([`verify_node`], delegating to the inner-layer [`VerifyGate`]), and
//! rollback ([`rollback_node`], delegating to
//! [`SessionCoordinator::rollback_to`]).
//!
//! Decoupling invariant: this module contains no references to orchestration
//! skills. Verify failure is returned to the agent as a tool result; the agent
//! decides escalation based on its active flow.

use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::coordinator::SessionCoordinator;
use super::hooks::{NoHooks, SessionHooks};
use super::node::{Node, NodeContract, NodeId, NodeStatus};
use super::session::SessionStatus;
use super::verify_gate::VerifyGate;

/// Result of a `verify_node` call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeVerifyResult {
    pub status: NodeStatus,
    pub retry_count: u32,
    pub failure_reason: Option<String>,
}

/// Result of a `rollback_node` call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeRollbackResult {
    pub rolled_back_to: NodeId,
    pub removed_nodes: Vec<NodeId>,
}

/// Coordinates the node-level state machine. Holds a shared coordinator
/// (same `Arc<RwLock<SessionCoordinator>>` as the inner-layer `VerifyGate`)
/// and delegates verification to `VerifyGate`.
pub struct NodeRuntime {
    coordinator: Arc<RwLock<SessionCoordinator>>,
    verify_gate: Arc<VerifyGate>,
    auto_retry_max: u32,
    hooks: Arc<dyn SessionHooks>,
}

impl NodeRuntime {
    pub fn new(
        coordinator: Arc<RwLock<SessionCoordinator>>,
        verify_gate: Arc<VerifyGate>,
        auto_retry_max: u32,
        hooks: Arc<dyn SessionHooks>,
    ) -> Self {
        Self {
            coordinator,
            verify_gate,
            auto_retry_max,
            hooks,
        }
    }

    /// Convenience constructor with `NoHooks`.
    pub fn new_with_default_hooks(
        coordinator: Arc<RwLock<SessionCoordinator>>,
        verify_gate: Arc<VerifyGate>,
        auto_retry_max: u32,
    ) -> Self {
        Self::new(coordinator, verify_gate, auto_retry_max, Arc::new(NoHooks))
    }

    /// Begin a new verifiable work unit (node).
    ///
    /// Precondition: the current node (if any) must be `Verified`. Creates a
    /// new node in `Running` status, linked to the current turn as its
    /// `start_turn_id`.
    pub async fn begin_node(
        &self,
        goal: String,
        verify_commands: Vec<String>,
        expected_files: Vec<String>,
    ) -> Result<NodeId> {
        let mut coord = self
            .coordinator
            .write()
            .map_err(|e| anyhow::anyhow!("coordinator write lock: {e}"))?;

        // Precondition: current node must be Verified or absent.
        if let Some(node) = coord.current_node() {
            if node.status != NodeStatus::Verified {
                anyhow::bail!(
                    "cannot begin_node: current node {:?} is not Verified",
                    node.status
                );
            }
        }

        let start_turn_id = coord.current_turn_id().unwrap_or("turn-0").to_string();
        let node_index = coord.node_states().len() + 1;
        let node_id = format!("n{}", node_index);
        let node = Node {
            id: node_id.clone(),
            contract: NodeContract {
                goal,
                verify_commands,
                expected_files,
            },
            status: NodeStatus::Running,
            start_turn_id,
            retry_count: 0,
            verify_log_path: String::new(),
            created_at: chrono::Utc::now().to_rfc3339(),
        };

        self.hooks.pre_node(&node);
        coord.add_node(node).context("add_node failed")?;
        Ok(node_id)
    }

    /// Verify the current node by executing its verify commands via the
    /// inner-layer `VerifyGate`.
    ///
    /// On success: node transitions to `Verified`.
    /// On failure (retry < max): node transitions to `Failed`, agent can
    /// self-correct and retry.
    /// On failure (retry >= max): session transitions to `Failed`,
    /// escalation returned to agent.
    pub async fn verify_node(&self) -> Result<NodeVerifyResult> {
        // Read current node's contract (short-lived read lock).
        let (node_id, commands, expected_files) = {
            let coord = self
                .coordinator
                .read()
                .map_err(|e| anyhow::anyhow!("coordinator read lock: {e}"))?;
            let node = coord
                .current_node()
                .ok_or_else(|| anyhow::anyhow!("verify_node: no current node"))?;
            if node.status != NodeStatus::Running && node.status != NodeStatus::Failed {
                anyhow::bail!(
                    "verify_node: current node must be Running or Failed, got {:?}",
                    node.status
                );
            }
            (
                node.id.clone(),
                node.contract.verify_commands.clone(),
                node.contract.expected_files.clone(),
            )
        };

        // Set status to Verifying.
        {
            let mut coord = self
                .coordinator
                .write()
                .map_err(|e| anyhow::anyhow!("coordinator write lock: {e}"))?;
            coord
                .update_node_status(&node_id, NodeStatus::Verifying)
                .context("set Verifying status")?;
        }

        // Run verify via the inner-layer VerifyGate.
        let expected_paths: Vec<PathBuf> = expected_files.iter().map(PathBuf::from).collect();
        let result = self
            .verify_gate
            .verify_and_complete(commands, expected_paths)
            .await
            .context("verify_and_complete failed")?;

        // Handle result (write lock).
        let mut coord = self
            .coordinator
            .write()
            .map_err(|e| anyhow::anyhow!("coordinator write lock: {e}"))?;

        if result.success {
            // verify_and_complete sets session to Completed (turn-level);
            // undo that for node-level verify and set node to Verified.
            coord
                .set_status(SessionStatus::InProgress)
                .context("reset session status after node verify")?;
            coord
                .update_node_status(&node_id, NodeStatus::Verified)
                .context("set Verified status")?;
            let node = coord.current_node().expect("node just updated").clone();
            self.hooks.post_node(&node, &result);
            Ok(NodeVerifyResult {
                status: NodeStatus::Verified,
                retry_count: 0,
                failure_reason: None,
            })
        } else {
            coord
                .update_node_status(&node_id, NodeStatus::Failed)
                .context("set Failed status")?;
            coord
                .increment_node_retry(&node_id)
                .context("increment retry")?;
            let retry_count = coord.current_node().map(|n| n.retry_count).unwrap_or(0);
            let node = coord.current_node().expect("node just updated").clone();
            let failure_reason = result.fail_reason.as_ref().map(|f| format!("{f:?}"));

            if retry_count >= self.auto_retry_max {
                coord
                    .set_status(SessionStatus::Failed)
                    .context("set session Failed (retry exhausted)")?;
            }
            self.hooks.post_node(&node, &result);
            Ok(NodeVerifyResult {
                status: NodeStatus::Failed,
                retry_count,
                failure_reason,
            })
        }
    }

    /// Roll back to the most recent `Verified` node, removing all nodes after
    /// it and restoring the workspace to the verified node's state.
    ///
    /// Delegates workspace restoration to
    /// [`SessionCoordinator::rollback_to`] and node cleanup to
    /// [`SessionCoordinator::truncate_nodes_after`].
    pub async fn rollback_node(&self) -> Result<NodeRollbackResult> {
        let mut coord = self
            .coordinator
            .write()
            .map_err(|e| anyhow::anyhow!("coordinator write lock: {e}"))?;

        // Find last Verified node.
        let verified_node_id = coord
            .node_states()
            .iter()
            .rev()
            .find(|n| n.status == NodeStatus::Verified)
            .map(|n| n.id.clone())
            .ok_or_else(|| anyhow::anyhow!("no verified node to roll back to"))?;

        // Find the first node after the verified node; its start_turn_id is
        // the workspace rollback target.
        let rollback_turn = {
            let pos = coord
                .node_states()
                .iter()
                .position(|n| n.id == verified_node_id);
            match pos {
                Some(idx) if idx + 1 < coord.node_states().len() => {
                    coord.node_states()[idx + 1].start_turn_id.clone()
                }
                _ => {
                    anyhow::bail!(
                        "no nodes after verified node {:?} to roll back",
                        verified_node_id
                    );
                }
            }
        };

        // Restore workspace to the rollback turn.
        coord
            .rollback_to(&rollback_turn, &*self.hooks)
            .context("workspace rollback_to failed")?;

        // Remove nodes after the verified node.
        let removed = coord
            .truncate_nodes_after(&verified_node_id)
            .context("truncate_nodes_after failed")?;

        Ok(NodeRollbackResult {
            rolled_back_to: verified_node_id,
            removed_nodes: removed,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec_session::session::SessionSource;
    use crate::exec_session::verify_gate::{CommandExecutor, CommandRun};
    use crate::tools::checkpoint_store::CheckpointStore;
    use async_trait::async_trait;
    use tempfile::TempDir;

    use std::path::Path;

    /// Mock command executor with a configurable exit code.
    struct MockExecutor {
        exit_code: i32,
    }

    #[async_trait]
    impl CommandExecutor for MockExecutor {
        async fn execute(&self, command: &str, _project_root: &Path) -> Result<CommandRun> {
            Ok(CommandRun {
                cmd: command.to_string(),
                exit_code: Some(self.exit_code),
                stdout: String::new(),
                stderr: if self.exit_code != 0 {
                    "command failed".to_string()
                } else {
                    String::new()
                },
            })
        }
    }

    /// Test fixture: creates a coordinator, verify gate, and node runtime in
    /// a temp directory.
    struct TestSetup {
        runtime: NodeRuntime,
        coord: Arc<RwLock<SessionCoordinator>>,
        _dir: TempDir,
    }

    impl TestSetup {
        fn new(exit_code: i32) -> Self {
            let dir = TempDir::new().unwrap();
            let store = Arc::new(CheckpointStore::new(dir.path()));
            let coord = SessionCoordinator::new(
                "es-test".into(),
                SessionSource::AgentSelf,
                dir.path(),
                store,
            )
            .unwrap();
            let coord = Arc::new(RwLock::new(coord));
            let executor = Arc::new(MockExecutor { exit_code });
            let gate = Arc::new(VerifyGate::new_with_default_hooks(
                Arc::clone(&coord),
                executor,
            ));
            let runtime = NodeRuntime::new_with_default_hooks(Arc::clone(&coord), gate, 2);
            Self {
                runtime,
                coord,
                _dir: dir,
            }
        }

        fn begin_turn(&self) {
            self.coord.write().unwrap().begin_turn().unwrap();
        }
    }

    #[tokio::test]
    async fn begin_node_creates_running_node() {
        let setup = TestSetup::new(0);
        setup.begin_turn(); // turn-0

        let node_id = setup
            .runtime
            .begin_node("test goal".into(), vec!["echo ok".into()], vec![])
            .await
            .unwrap();

        assert_eq!(node_id, "n1");
        let coord = setup.coord.read().unwrap();
        let node = coord.current_node().unwrap();
        assert_eq!(node.id, "n1");
        assert_eq!(node.status, NodeStatus::Running);
        assert_eq!(node.start_turn_id, "turn-0");
    }

    #[tokio::test]
    async fn begin_node_rejected_when_current_not_verified() {
        let setup = TestSetup::new(0);
        setup.begin_turn();
        setup
            .runtime
            .begin_node("goal1".into(), vec!["echo ok".into()], vec![])
            .await
            .unwrap();

        // Current node is Running; begin_node should fail.
        let err = setup
            .runtime
            .begin_node("goal2".into(), vec!["echo ok".into()], vec![])
            .await
            .unwrap_err();
        assert!(format!("{err}").contains("not Verified"));
    }

    #[tokio::test]
    async fn verify_node_success_transitions_to_verified() {
        let setup = TestSetup::new(0); // exit 0 = success
        setup.begin_turn();
        setup
            .runtime
            .begin_node("goal".into(), vec!["echo ok".into()], vec![])
            .await
            .unwrap();

        let result = setup.runtime.verify_node().await.unwrap();
        assert_eq!(result.status, NodeStatus::Verified);
        assert_eq!(result.retry_count, 0);
        assert!(result.failure_reason.is_none());

        let coord = setup.coord.read().unwrap();
        assert_eq!(coord.current_node().unwrap().status, NodeStatus::Verified);
        // Session should NOT be Completed (that's turn-level).
        assert_eq!(coord.session().status, SessionStatus::InProgress);
    }

    #[tokio::test]
    async fn verify_node_failure_within_retry_budget() {
        let setup = TestSetup::new(1); // exit 1 = failure
        setup.begin_turn();
        setup
            .runtime
            .begin_node("goal".into(), vec!["echo fail".into()], vec![])
            .await
            .unwrap();

        let result = setup.runtime.verify_node().await.unwrap();
        assert_eq!(result.status, NodeStatus::Failed);
        assert_eq!(result.retry_count, 1); // first failure
        assert!(result.failure_reason.is_some());

        let coord = setup.coord.read().unwrap();
        assert_eq!(coord.current_node().unwrap().status, NodeStatus::Failed);
        assert_eq!(coord.session().status, SessionStatus::InProgress);
    }

    #[tokio::test]
    async fn verify_node_exceeds_retry_budget_escalates() {
        let setup = TestSetup::new(1); // always fails
        setup.begin_turn();
        setup
            .runtime
            .begin_node("goal".into(), vec!["echo fail".into()], vec![])
            .await
            .unwrap();

        // First failure (retry_count=1, < max=2).
        let r1 = setup.runtime.verify_node().await.unwrap();
        assert_eq!(r1.status, NodeStatus::Failed);
        assert_eq!(r1.retry_count, 1);

        // Second failure (retry_count=2, >= max=2 -> session.failed).
        let r2 = setup.runtime.verify_node().await.unwrap();
        assert_eq!(r2.status, NodeStatus::Failed);
        assert_eq!(r2.retry_count, 2);

        let coord = setup.coord.read().unwrap();
        assert_eq!(coord.session().status, SessionStatus::Failed);
    }

    #[tokio::test]
    async fn rollback_node_restores_to_verified() {
        let setup = TestSetup::new(0); // success
        setup.begin_turn(); // turn-0
        setup
            .runtime
            .begin_node("node1".into(), vec!["echo ok".into()], vec![])
            .await
            .unwrap();
        setup.runtime.verify_node().await.unwrap(); // n1 verified

        // Start a second node (will be removed by rollback).
        setup.begin_turn(); // turn-1
        setup
            .runtime
            .begin_node("node2".into(), vec!["echo ok".into()], vec![])
            .await
            .unwrap();

        let result = setup.runtime.rollback_node().await.unwrap();
        assert_eq!(result.rolled_back_to, "n1");
        assert_eq!(result.removed_nodes, vec!["n2".to_string()]);

        let coord = setup.coord.read().unwrap();
        assert_eq!(coord.node_states().len(), 1);
        assert_eq!(coord.current_node().unwrap().id, "n1");
        assert_eq!(coord.current_node().unwrap().status, NodeStatus::Verified);
    }

    #[tokio::test]
    async fn rollback_node_errors_without_verified() {
        let setup = TestSetup::new(0);
        setup.begin_turn();
        setup
            .runtime
            .begin_node("goal".into(), vec!["echo ok".into()], vec![])
            .await
            .unwrap();
        // Node is Running, not Verified.

        let err = setup.runtime.rollback_node().await.unwrap_err();
        assert!(format!("{err}").contains("no verified node"));
    }

    #[tokio::test]
    async fn begin_node_after_verified_starts_new_node() {
        let setup = TestSetup::new(0);
        setup.begin_turn(); // turn-0
        setup
            .runtime
            .begin_node("node1".into(), vec!["echo ok".into()], vec![])
            .await
            .unwrap();
        setup.runtime.verify_node().await.unwrap(); // n1 verified

        setup.begin_turn(); // turn-1
        let n2 = setup
            .runtime
            .begin_node("node2".into(), vec!["echo ok".into()], vec![])
            .await
            .unwrap();
        assert_eq!(n2, "n2");

        let coord = setup.coord.read().unwrap();
        assert_eq!(coord.node_states().len(), 2);
        assert_eq!(coord.current_node().unwrap().id, "n2");
    }
}
