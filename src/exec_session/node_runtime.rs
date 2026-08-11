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
use super::hooks::{NoHooks, SessionHooks, VerifyFailure};
use super::node::{Node, NodeContract, NodeId, NodeStatus};
use super::session::SessionStatus;
use super::verify_gate::{VerifyGate, VerifyResult};
use crate::org_graph::NodeType;

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

/// exec_session::VerifyFailure → org_graph::VerifyFailureKind 投影。
/// 投影规则：CommandFailed 保留 exit_code + stderr（丢 command 字符串，retry
/// 决策只需 exit_code 语义）；BoundaryViolation 保留 unexpected_files。
fn project_failure(f: &VerifyFailure) -> crate::org_graph::VerifyFailureKind {
    match f {
        VerifyFailure::CommandFailed { exit_code, stderr, .. } => {
            crate::org_graph::VerifyFailureKind::CommandFailed {
                exit_code: *exit_code,
                stderr: stderr.clone(),
            }
        }
        VerifyFailure::BoundaryViolation { unexpected_files } => {
            crate::org_graph::VerifyFailureKind::BoundaryViolation {
                unexpected_files: unexpected_files.clone(),
            }
        }
    }
}

/// exec_session::VerifyResult → org_graph::VerifyOutcome 投影。
fn project_outcome(result: &VerifyResult) -> crate::org_graph::VerifyOutcome {
    crate::org_graph::VerifyOutcome::from_parts(
        result.success,
        result.fail_reason.as_ref().map(project_failure),
    )
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
            // pilot 修复（D5）：成功也写 WorkState（Success{fail_reason:None}），
            // 保证字段级强制在成功+失败两路径都可验证。
            let outcome = project_outcome(&result);
            coord
                .work_state_mut()
                .set_verify_result(NodeType::Verification, outcome)
                .context("write verify_result (success) into WorkState")?;
            coord
                .capture_current_work_state()
                .context("persist work_state after verify_node")?;
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

            // pilot 修复（D5）：把强类型 VerifyResult 投影写入 WorkState（受权限强制），
            // 再从 WorkState 读回组装兼容期 failure_reason。retry 决策保持 count-based
            // 不变（auto_retry_max）；强类型 VerifyOutcome 入 WorkState 是本期交付物，
            // 枚举感知 retry 是将来 consumer 的范围。
            let outcome = project_outcome(&result);
            coord
                .work_state_mut()
                .set_verify_result(NodeType::Verification, outcome)
                .context("write verify_result into WorkState")?;
            coord
                .capture_current_work_state()
                .context("persist work_state after verify_node")?;
            // 从 WorkState 读回（验证读写闭环 + 权限强制生效）。
            let outcome_ref = coord
                .work_state()
                .verify_result(NodeType::Verification)
                .context("read verify_result from WorkState")?
                .with_context(|| "verify_result missing after set_verify_result")?;
            // 兼容期：String 字段源头改为从强类型枚举转 debug string。
            let failure_reason = outcome_ref
                .fail_reason
                .as_ref()
                .map(|f| format!("{f:?}"));

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
    use crate::org_graph::NodeType;
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

    #[tokio::test]
    async fn verify_node_failure_writes_structured_outcome_to_work_state() {
        // pilot D5 硬约束：verify 失败后 WorkState.verify_result 必须是强类型
        // VerifyOutcome（非 format!("{f:?}") 文本）。retry 决策读 VerifyFailureKind 枚举分支。
        let setup = TestSetup::new(1); // exit 1 = failure
        setup.begin_turn();
        setup
            .runtime
            .begin_node("goal".into(), vec!["echo ok".into()], vec![])
            .await
            .unwrap();

        let result = setup.runtime.verify_node().await.unwrap();
        assert_eq!(result.status, NodeStatus::Failed);

        // 核心断言：WorkState 持有强类型 VerifyOutcome，retry 决策可读枚举分支。
        let coord = setup.coord.read().unwrap();
        let outcome_ref = coord
            .work_state()
            .verify_result(NodeType::Verification)
            .expect("Verification may read")
            .expect("verify_result must be populated after failed verify_node");
        assert!(!outcome_ref.success);
        match &outcome_ref.fail_reason {
            Some(crate::org_graph::VerifyFailureKind::CommandFailed { exit_code, stderr }) => {
                assert_eq!(*exit_code, Some(1));
                assert!(stderr.contains("command failed"));
            }
            other => panic!("expected CommandFailed, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn verify_node_failure_reason_string_comes_from_work_state() {
        // 兼容期：NodeVerifyResult.failure_reason 仍为 String，但源头改为
        // 从 WorkState 强类型枚举读回转 debug string（而非 format!("{f:?}")）。
        let setup = TestSetup::new(1);
        setup.begin_turn();
        setup
            .runtime
            .begin_node("goal".into(), vec!["echo ok".into()], vec![])
            .await
            .unwrap();
        let result = setup.runtime.verify_node().await.unwrap();
        assert!(result.failure_reason.is_some());
        // String 内容应反映 CommandFailed 枚举（debug 格式）。
        let reason = result.failure_reason.unwrap();
        assert!(reason.contains("CommandFailed"));
    }

    #[tokio::test]
    async fn verify_node_success_clears_fail_reason_in_work_state() {
        // 成功路径：WorkState.verify_result = Some(Success{fail_reason:None})，
        // failure_reason String 为 None。
        let setup = TestSetup::new(0);
        setup.begin_turn();
        setup
            .runtime
            .begin_node("goal".into(), vec!["echo ok".into()], vec![])
            .await
            .unwrap();
        let result = setup.runtime.verify_node().await.unwrap();
        assert_eq!(result.status, NodeStatus::Verified);
        assert!(result.failure_reason.is_none());
        let coord = setup.coord.read().unwrap();
        let outcome = coord
            .work_state()
            .verify_result(NodeType::Verification)
            .expect("Verification may read")
            .expect("success also writes verify_result");
        assert!(outcome.success);
        assert!(outcome.fail_reason.is_none());
    }

    #[test]
    fn set_verify_result_rejects_unauthorized_node_type_at_pilot_site() {
        // pilot 写场景的字段级强制：直接调 WorkState API 验证非授权节点被拦。
        // 这保证 D5「字段级强制有真实场景可拦」承诺落地——
        // 若 field_perms 矩阵被错误放宽，本测试会红。
        let mut state = crate::org_graph::WorkState::default();
        let err = state
            .set_verify_result(
                crate::org_graph::NodeType::Explore,
                crate::org_graph::VerifyOutcome {
                    success: true,
                    fail_reason: None,
                },
            )
            .expect_err("Explore must not write verify_result");
        assert!(matches!(
            err,
            crate::agent::coordinator::CoordinatorError::ContractViolation { .. }
        ));
    }

    #[tokio::test]
    async fn verify_node_persists_work_state_to_checkpoint_after_failure() {
        // spec SHALL: 写入结构化工作状态后随 turn 检查点持久化。
        // verify_node 失败后，当前 turn 的 work_state.json 必须落盘且含强类型 outcome。
        let setup = TestSetup::new(1); // exit 1 = failure
        setup.begin_turn();
        setup
            .runtime
            .begin_node("goal".into(), vec!["echo ok".into()], vec![])
            .await
            .unwrap();
        setup.runtime.verify_node().await.unwrap();

        let coord = setup.coord.read().unwrap();
        let current_turn = coord.current_turn_id().expect("active turn").to_string();
        let checkpoint_turn_id = coord
            .session()
            .turns
            .iter()
            .find(|t| t.turn_id == current_turn)
            .expect("active turn")
            .checkpoint_turn_id
            .clone();
        let ws_path = coord
            .project_root()
            .join(".wgenty-code")
            .join("checkpoints")
            .join(&checkpoint_turn_id)
            .join("work_state.json");
        assert!(
            ws_path.exists(),
            "work_state.json must be persisted after verify_node: {}",
            ws_path.display()
        );
        let on_disk: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(&ws_path).unwrap(),
        )
        .unwrap();
        assert_eq!(
            on_disk["verify_result"]["success"], false,
            "persisted verify_result.success must be false: {on_disk}"
        );
        assert!(
            on_disk["verify_result"].get("fail_reason").is_some(),
            "persisted verify_result must carry structured fail_reason: {on_disk}"
        );
    }

    #[tokio::test]
    async fn verify_node_retry_overwrites_work_state_in_place_same_turn() {
        // 同 turn 内第二次 verify_node 覆盖 verify_result（不重置 turn、不丢字段）。
        // retry 是 count-based，不走 begin_turn；WorkState 在同 turn 内自然保留并被覆盖。
        let setup = TestSetup::new(1); // exit 1 = always failure
        setup.begin_turn();
        setup
            .runtime
            .begin_node("goal".into(), vec!["echo ok".into()], vec![])
            .await
            .unwrap();
        setup.runtime.verify_node().await.unwrap(); // writes first failure outcome
        {
            let coord = setup.coord.read().unwrap();
            let o = coord
                .work_state()
                .verify_result(NodeType::Verification)
                .unwrap()
                .unwrap();
            assert!(!o.success);
        }
        // Second verify in the same turn — overwrites verify_result in place.
        setup.runtime.verify_node().await.unwrap();
        let coord = setup.coord.read().unwrap();
        let o = coord
            .work_state()
            .verify_result(NodeType::Verification)
            .unwrap()
            .unwrap();
        assert!(!o.success, "second failure overwrites the first");
        assert_eq!(
            coord.current_turn_id(),
            Some("turn-0"),
            "same turn — no begin_turn between retries"
        );
    }

    // 端到端验证测试（Task 6 集成验证）：GREEN on first run。
    // 验证 Task 4 pilot 修复端到端成立——非新增实现，故无 RED 阶段。
    #[tokio::test]
    async fn pilot_end_to_end_retry_reads_structured_failure_kind() {
        // 端到端：verify 失败 → WorkState 写入强类型 → retry 决策读 VerifyFailureKind
        // 分支（CommandFailed vs BoundaryViolation）做不同处理。
        let setup = TestSetup::new(1); // CommandFailed
        setup.begin_turn();
        setup
            .runtime
            .begin_node("goal".into(), vec!["echo ok".into()], vec![])
            .await
            .unwrap();
        setup.runtime.verify_node().await.unwrap();

        // 模拟 retry 决策点：读 WorkState.verify_result 拿强类型分支。
        let coord = setup.coord.read().unwrap();
        let outcome = coord
            .work_state()
            .verify_result(NodeType::Verification)
            .unwrap()
            .unwrap();
        // 路由判定：CommandFailed → 回到代码生成（pilot 文本不再参与判定）。
        match &outcome.fail_reason {
            Some(crate::org_graph::VerifyFailureKind::CommandFailed { .. }) => {
                // 命中「回到代码生成」分支
            }
            Some(crate::org_graph::VerifyFailureKind::BoundaryViolation { .. }) => {
                panic!("expected CommandFailed for exit 1, got BoundaryViolation");
            }
            None => panic!("expected failure, got success"),
        }
    }
}
