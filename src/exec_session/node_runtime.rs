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
use tokio::sync::Mutex as AsyncMutex;

use super::coordinator::SessionCoordinator;
use super::hooks::{NoHooks, SessionHooks, VerifyFailure};
use super::node::{Node, NodeContract, NodeId, NodeStatus};
use super::session::SessionStatus;
use super::verification_profile::VerificationProfile;
use super::verify_gate::{VerifyGate, VerifyResult};
use super::work_graph::{next_step, WorkGraphStep};
use crate::org_graph::{select_work_graph, WorkGraphRequest};
use crate::org_graph::{
    AuditCommandRun, Budget, CompileResult, GeneratedDiff, GraphAuditAnchor, GraphAuditCommands,
    GraphAuditEvent, GraphAuditKind, GraphAuditProfile, GraphAuditRoute, HumanReview, NodeType,
    TestResult,
};

const AUDIT_STDERR_LIMIT_BYTES: usize = 8_192;
const AUDIT_STDERR_TRUNCATION_MARKER: &str = "\n...[stderr truncated]";

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

/// Result produced by a complete fixed work-graph pass.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkGraphRunResult {
    pub next_step: WorkGraphStep,
}

/// The verification path selected from the persisted node contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NodeVerificationOutcome {
    Legacy(NodeVerifyResult),
    WorkGraph(WorkGraphRunResult),
}

/// Coordinates the node-level state machine. Holds a shared coordinator
/// (same `Arc<RwLock<SessionCoordinator>>` as the inner-layer `VerifyGate`)
/// and delegates verification to `VerifyGate`.
pub struct NodeRuntime {
    coordinator: Arc<RwLock<SessionCoordinator>>,
    verify_gate: Arc<VerifyGate>,
    work_graph_gate: AsyncMutex<()>,
    auto_retry_max: u32,
    hooks: Arc<dyn SessionHooks>,
}

/// exec_session::VerifyFailure → org_graph::VerifyFailureKind 投影。
/// 投影规则：CommandFailed 保留 exit_code + stderr（丢 command 字符串，retry
/// 决策只需 exit_code 语义）；BoundaryViolation 保留 unexpected_files。
fn project_failure(f: &VerifyFailure) -> crate::org_graph::VerifyFailureKind {
    match f {
        VerifyFailure::CommandFailed {
            exit_code, stderr, ..
        } => crate::org_graph::VerifyFailureKind::CommandFailed {
            exit_code: *exit_code,
            stderr: stderr.clone(),
        },
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

/// Join non-empty stderr streams so the anchor state retains actionable
/// diagnostics without depending on the execution-layer command type.
fn collect_stderr(runs: &[crate::exec_session::CommandRun]) -> String {
    runs.iter()
        .filter_map(|run| (!run.stderr.is_empty()).then_some(run.stderr.as_str()))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Work-graph identity cloned from coordinator state before command execution.
struct WorkGraphAuditContext {
    node_id: String,
    attempt: u32,
}

fn numeric_node_id(node_id: &str) -> Option<u64> {
    node_id
        .strip_prefix('n')
        .filter(|suffix| !suffix.is_empty())
        .and_then(|suffix| suffix.parse().ok())
}

fn next_node_id(coordinator: &SessionCoordinator) -> Result<NodeId> {
    let max_id = coordinator
        .node_states()
        .iter()
        .map(|node| node.id.as_str())
        .chain(
            coordinator
                .work_state()
                .graph_audit()
                .iter()
                .map(|event| event.node_id.as_str()),
        )
        .filter_map(numeric_node_id)
        .max()
        .unwrap_or(0);
    let next_id = max_id.checked_add(1).context("node id space exhausted")?;
    Ok(format!("n{next_id}"))
}

fn project_audit_profile(profile: VerificationProfile) -> GraphAuditProfile {
    match profile {
        VerificationProfile::None => GraphAuditProfile::None,
        VerificationProfile::Rust => GraphAuditProfile::Rust,
    }
}

fn project_audit_route(step: WorkGraphStep) -> GraphAuditRoute {
    match step {
        WorkGraphStep::RootCause => GraphAuditRoute::RootCause,
        WorkGraphStep::Implement => GraphAuditRoute::Implement,
        WorkGraphStep::CompileAnchor => GraphAuditRoute::CompileAnchor,
        WorkGraphStep::TestAnchor => GraphAuditRoute::TestAnchor,
        WorkGraphStep::VerifyGate => GraphAuditRoute::VerifyGate,
        WorkGraphStep::AwaitHumanReview => GraphAuditRoute::HumanReview,
        WorkGraphStep::Complete => GraphAuditRoute::Complete,
        WorkGraphStep::Escalate => GraphAuditRoute::Escalate,
    }
}

fn truncate_audit_stderr(stderr: &str) -> String {
    if stderr.len() <= AUDIT_STDERR_LIMIT_BYTES {
        return stderr.to_string();
    }

    let mut prefix_len = AUDIT_STDERR_LIMIT_BYTES - AUDIT_STDERR_TRUNCATION_MARKER.len();
    while !stderr.is_char_boundary(prefix_len) {
        prefix_len -= 1;
    }
    format!(
        "{}{}",
        &stderr[..prefix_len],
        AUDIT_STDERR_TRUNCATION_MARKER
    )
}

fn project_audit_commands(runs: &[crate::exec_session::CommandRun]) -> Vec<AuditCommandRun> {
    runs.iter()
        .map(|run| AuditCommandRun {
            command: run.cmd.clone(),
            exit_code: run.exit_code,
            stderr: truncate_audit_stderr(&run.stderr),
        })
        .collect()
}

fn base_audit_event(context: &WorkGraphAuditContext, kind: GraphAuditKind) -> GraphAuditEvent {
    GraphAuditEvent {
        node_id: context.node_id.clone(),
        attempt: context.attempt,
        kind,
        anchor: None,
        commands: Vec::new(),
        route: None,
        profile: None,
        resolved_commands: None,
        budget: None,
        timestamp: chrono::Utc::now().to_rfc3339(),
    }
}

fn anchor_audit_event(
    context: &WorkGraphAuditContext,
    anchor: GraphAuditAnchor,
    commands: Vec<AuditCommandRun>,
) -> GraphAuditEvent {
    let mut event = base_audit_event(context, GraphAuditKind::AnchorCompleted);
    event.anchor = Some(anchor);
    event.commands = commands;
    event
}

fn route_audit_event(
    context: &WorkGraphAuditContext,
    route: WorkGraphStep,
    budget: Option<Budget>,
) -> GraphAuditEvent {
    let mut event = base_audit_event(context, GraphAuditKind::RouteSelected);
    event.route = Some(project_audit_route(route));
    event.budget = budget;
    event
}

fn profile_audit_event(
    context: &WorkGraphAuditContext,
    profile: GraphAuditProfile,
    resolved_commands: GraphAuditCommands,
) -> GraphAuditEvent {
    let mut event = base_audit_event(context, GraphAuditKind::ProfileResolved);
    event.profile = Some(profile);
    event.resolved_commands = Some(resolved_commands);
    event
}

/// Charge one coordinator-owned work-graph iteration after a failed anchor.
fn consume_iteration_budget(coord: &mut SessionCoordinator) -> Result<()> {
    let mut budget = coord
        .work_state()
        .budget(NodeType::GeneralPurpose)
        .context("read work graph budget")?
        .cloned()
        .context("work graph budget must be initialized")?;
    budget.iter_used = budget.iter_used.saturating_add(1);
    coord
        .work_state_mut()
        .set_budget(NodeType::GeneralPurpose, budget)
        .context("consume work graph iteration budget")
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
            work_graph_gate: AsyncMutex::new(()),
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

    /// Run the fixed compile → test → verify graph using real command output.
    /// Each anchor is persisted before its structured result determines the
    /// next edge. Callers supply commands; the runtime, not an agent report,
    /// is the source of truth for their outcome.
    ///
    /// Requires a current node already persisted by [`Self::begin_node`] or
    /// [`Self::begin_node_with_anchors`] so every audit event has a real node
    /// identity.
    pub async fn run_work_graph(
        &self,
        compile_commands: Vec<String>,
        test_commands: Vec<String>,
        verify_commands: Vec<String>,
        expected_files: Vec<String>,
    ) -> Result<WorkGraphRunResult> {
        // Serialize complete passes without retaining the coordinator lock
        // across external command awaits. This keeps attempt allocation and
        // every state transition for a node in one non-interleaved sequence.
        let _pass_guard = self.work_graph_gate.lock().await;
        let audit_context = self.prepare_work_graph_pass()?;
        let compile_runs = self
            .verify_gate
            .run_anchor_commands(&compile_commands)
            .await
            .context("run compile anchor commands")?;
        let compile_result = CompileResult {
            ok: compile_runs.iter().all(|run| run.exit_code == Some(0)),
            stderr: collect_stderr(&compile_runs),
        };
        let step = self.record_compile_result(&audit_context, compile_result, &compile_runs)?;
        if step != WorkGraphStep::TestAnchor {
            return self.complete_work_graph_route(step);
        }

        let test_runs = self
            .verify_gate
            .run_anchor_commands(&test_commands)
            .await
            .context("run test anchor commands")?;
        let test_result = TestResult {
            pass: test_runs.iter().all(|run| run.exit_code == Some(0)),
            failed_cases: test_runs
                .iter()
                .filter(|run| run.exit_code != Some(0))
                .map(|run| run.cmd.clone())
                .collect(),
        };
        let step = self.record_test_result(&audit_context, test_result, &test_runs)?;
        if step != WorkGraphStep::VerifyGate {
            return self.complete_work_graph_route(step);
        }

        let expected_paths = expected_files.iter().map(PathBuf::from).collect();
        let result = self
            .verify_gate
            .verify_for_work_graph(verify_commands, expected_paths)
            .await
            .context("run final verification gate")?;
        let outcome = project_outcome(&result);
        let mut coord = self
            .coordinator
            .write()
            .map_err(|e| anyhow::anyhow!("coordinator write lock: {e}"))?;
        let changed_files: Vec<String> = result
            .actual_changed_files
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect();
        coord
            .work_state_mut()
            .set_generated_diff(
                NodeType::GeneralPurpose,
                GeneratedDiff {
                    summary: format!("{} files observed by verification", changed_files.len()),
                    files: changed_files,
                },
            )
            .context("record workspace diff from verification")?;
        coord
            .work_state_mut()
            .set_verify_result(NodeType::Verification, outcome)
            .context("record verification anchor result")?;
        if matches!(
            result.fail_reason.as_ref(),
            Some(VerifyFailure::CommandFailed { .. })
        ) {
            consume_iteration_budget(&mut coord)?;
        }
        coord
            .work_state_mut()
            .append_graph_audit(anchor_audit_event(
                &audit_context,
                GraphAuditAnchor::Verify,
                project_audit_commands(&result.commands_run),
            ));
        coord
            .capture_current_work_state()
            .context("persist verification anchor audit event")?;
        let next_step = next_step(coord.work_state()).context("route verification result")?;
        let budget = coord
            .work_state()
            .budget(NodeType::GeneralPurpose)
            .context("read verification route budget")?
            .cloned();
        coord.work_state_mut().append_graph_audit(route_audit_event(
            &audit_context,
            next_step,
            budget,
        ));
        coord
            .capture_current_work_state()
            .context("persist verification route audit event")?;
        drop(coord);
        self.complete_work_graph_route(next_step)
    }

    /// Finish a persisted, code-owned route and synchronize any terminal edge
    /// through the VerifyGate's durable session/log transition.
    fn complete_work_graph_route(&self, next_step: WorkGraphStep) -> Result<WorkGraphRunResult> {
        match next_step {
            WorkGraphStep::Complete => self
                .verify_gate
                .mark_completed()
                .context("synchronize work graph completion status")?,
            WorkGraphStep::Escalate => {
                self.verify_gate
                    .mark_failed()
                    .context("synchronize work graph escalation status")?;
                let mut coord = self
                    .coordinator
                    .write()
                    .map_err(|e| anyhow::anyhow!("coordinator write lock: {e}"))?;
                let node_id = coord
                    .current_node()
                    .context("work graph escalation requires a current node")?
                    .id
                    .clone();
                coord
                    .update_node_status(&node_id, NodeStatus::Failed)
                    .context("mark escalated work graph node failed")?;
            }
            _ => {}
        }
        Ok(WorkGraphRunResult { next_step })
    }

    /// Synchronizes an out-of-band static-route failure with the durable node
    /// and verify-gate lifecycle. RootCause children use this when they reach a
    /// terminal state without publishing their required handoff.
    pub(crate) fn escalate_current_work_graph(&self) -> Result<()> {
        self.complete_work_graph_route(WorkGraphStep::Escalate)
            .context("synchronize root-cause terminal escalation")?;
        Ok(())
    }

    /// Record an authenticated external HumanReview decision for the current
    /// graph. This is intentionally not an agent-facing tool: callers must
    /// authenticate the human outside the model tool channel.
    pub fn record_human_review(&self, review: HumanReview) -> Result<WorkGraphRunResult> {
        let mut coord = self
            .coordinator
            .write()
            .map_err(|e| anyhow::anyhow!("coordinator write lock: {e}"))?;
        let current_node = coord
            .current_node()
            .context("human review requires a persisted current node")?;
        let node_id = current_node.id.clone();
        let requires_review = coord
            .work_state()
            .selected_work_graph()
            .is_some_and(|plan| {
                plan.nodes
                    .iter()
                    .any(|node| node.role == NodeType::HumanReview)
            });
        if !requires_review {
            anyhow::bail!("current Work-Graph does not include a human-review gate");
        }
        let verified = coord
            .work_state()
            .verify_result(NodeType::HumanReview)
            .context("read verification outcome for human review")?
            .is_some_and(|outcome| outcome.success);
        if !verified {
            anyhow::bail!("human review requires a successful external verification anchor");
        }
        coord
            .work_state_mut()
            .set_human_review(NodeType::HumanReview, review)
            .context("record authenticated human review")?;
        let next_step = next_step(coord.work_state()).context("route human review decision")?;
        let attempt = coord
            .work_state()
            .graph_audit()
            .iter()
            .filter(|event| event.node_id == node_id)
            .map(|event| event.attempt)
            .max()
            .unwrap_or(1);
        let budget = coord
            .work_state()
            .budget(NodeType::GeneralPurpose)
            .context("read human review route budget")?
            .cloned();
        coord.work_state_mut().append_graph_audit(route_audit_event(
            &WorkGraphAuditContext { node_id, attempt },
            next_step,
            budget,
        ));
        coord
            .capture_current_work_state()
            .context("persist human review decision")?;
        drop(coord);
        let result = self.complete_work_graph_route(next_step)?;
        if result.next_step == WorkGraphStep::Complete {
            let mut coord = self
                .coordinator
                .write()
                .map_err(|e| anyhow::anyhow!("coordinator write lock: {e}"))?;
            let node_id = coord
                .current_node()
                .context("human review completion requires a current node")?
                .id
                .clone();
            coord
                .set_status(SessionStatus::InProgress)
                .context("reset session after human review approval")?;
            coord
                .update_node_status(&node_id, NodeStatus::Verified)
                .context("mark node verified after human review approval")?;
        }
        Ok(result)
    }

    fn record_compile_result(
        &self,
        audit_context: &WorkGraphAuditContext,
        result: CompileResult,
        runs: &[crate::exec_session::CommandRun],
    ) -> Result<WorkGraphStep> {
        let failed = !result.ok;
        let mut coord = self
            .coordinator
            .write()
            .map_err(|e| anyhow::anyhow!("coordinator write lock: {e}"))?;
        coord
            .work_state_mut()
            .set_compile_result(NodeType::Verification, result)
            .context("record compile anchor result")?;
        if failed {
            consume_iteration_budget(&mut coord)?;
        }
        coord
            .work_state_mut()
            .append_graph_audit(anchor_audit_event(
                audit_context,
                GraphAuditAnchor::Compile,
                project_audit_commands(runs),
            ));
        coord
            .capture_current_work_state()
            .context("persist compile anchor audit event")?;
        let next_step = next_step(coord.work_state()).context("route compile anchor result")?;
        let budget = coord
            .work_state()
            .budget(NodeType::GeneralPurpose)
            .context("read compile route budget")?
            .cloned();
        coord.work_state_mut().append_graph_audit(route_audit_event(
            audit_context,
            next_step,
            budget,
        ));
        coord
            .capture_current_work_state()
            .context("persist compile route audit event")?;
        Ok(next_step)
    }

    fn record_test_result(
        &self,
        audit_context: &WorkGraphAuditContext,
        result: TestResult,
        runs: &[crate::exec_session::CommandRun],
    ) -> Result<WorkGraphStep> {
        let failed = !result.pass;
        let mut coord = self
            .coordinator
            .write()
            .map_err(|e| anyhow::anyhow!("coordinator write lock: {e}"))?;
        coord
            .work_state_mut()
            .set_test_result(NodeType::Verification, result)
            .context("record test anchor result")?;
        if failed {
            consume_iteration_budget(&mut coord)?;
        }
        coord
            .work_state_mut()
            .append_graph_audit(anchor_audit_event(
                audit_context,
                GraphAuditAnchor::Test,
                project_audit_commands(runs),
            ));
        coord
            .capture_current_work_state()
            .context("persist test anchor audit event")?;
        let next_step = next_step(coord.work_state()).context("route test anchor result")?;
        let budget = coord
            .work_state()
            .budget(NodeType::GeneralPurpose)
            .context("read test route budget")?
            .cloned();
        coord.work_state_mut().append_graph_audit(route_audit_event(
            audit_context,
            next_step,
            budget,
        ));
        coord
            .capture_current_work_state()
            .context("persist test route audit event")?;
        Ok(next_step)
    }

    fn prepare_work_graph_pass(&self) -> Result<WorkGraphAuditContext> {
        let mut coord = self
            .coordinator
            .write()
            .map_err(|e| anyhow::anyhow!("coordinator write lock: {e}"))?;
        coord
            .session()
            .current_turn_record()
            .context("run work graph requires an active turn/checkpoint")?;
        let node_id = coord
            .current_node()
            .map(|node| node.id.clone())
            .context("run work graph requires a persisted current node")?;
        let attempt = coord
            .work_state()
            .graph_audit()
            .iter()
            .filter(|event| {
                event.node_id == node_id && event.kind != GraphAuditKind::ProfileResolved
            })
            .map(|event| event.attempt)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        coord.work_state_mut().reset_for_work_graph_pass();
        if coord
            .work_state()
            .budget(NodeType::GeneralPurpose)
            .context("read work graph budget")?
            .is_none()
        {
            coord
                .work_state_mut()
                .set_budget(
                    NodeType::GeneralPurpose,
                    Budget {
                        max_iter: self.auto_retry_max,
                        iter_used: 0,
                        token_used: 0,
                    },
                )
                .context("initialize work graph budget")?;
        }
        coord
            .capture_current_work_state()
            .context("persist fresh work graph pass state")?;
        Ok(WorkGraphAuditContext { node_id, attempt })
    }

    /// Verify the current node using its persisted contract. Nodes without
    /// compile/test anchors retain the original verification-only behavior.
    pub async fn verify_current_node(&self) -> Result<NodeVerificationOutcome> {
        let (compile_commands, test_commands, verify_commands, expected_files) = {
            let coord = self
                .coordinator
                .read()
                .map_err(|e| anyhow::anyhow!("coordinator read lock: {e}"))?;
            let node = coord
                .current_node()
                .context("verify_current_node: no current node")?;
            (
                node.contract.compile_commands.clone(),
                node.contract.test_commands.clone(),
                node.contract.verify_commands.clone(),
                node.contract.expected_files.clone(),
            )
        };
        if compile_commands.is_empty() && test_commands.is_empty() {
            return self
                .verify_node()
                .await
                .map(NodeVerificationOutcome::Legacy);
        }
        let result = self
            .run_work_graph(
                compile_commands,
                test_commands,
                verify_commands,
                expected_files,
            )
            .await?;
        if result.next_step == WorkGraphStep::Complete {
            let mut coord = self
                .coordinator
                .write()
                .map_err(|e| anyhow::anyhow!("coordinator write lock: {e}"))?;
            let node_id = coord
                .current_node()
                .context("verify_current_node: current node disappeared")?
                .id
                .clone();
            // The final gate completes the turn-level session; a verified node
            // leaves the outer node lifecycle ready for a subsequent node.
            coord
                .set_status(SessionStatus::InProgress)
                .context("reset session after work graph success")?;
            coord
                .update_node_status(&node_id, NodeStatus::Verified)
                .context("mark node verified after work graph success")?;
        }
        Ok(NodeVerificationOutcome::WorkGraph(result))
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
        self.begin_node_with_anchors(
            goal,
            Vec::new(),
            Vec::new(),
            verify_commands,
            expected_files,
        )
        .await
    }

    /// Begin a node with optional deterministic compile and test anchors.
    pub async fn begin_node_with_anchors(
        &self,
        goal: String,
        compile_commands: Vec<String>,
        test_commands: Vec<String>,
        verify_commands: Vec<String>,
        expected_files: Vec<String>,
    ) -> Result<NodeId> {
        self.begin_node_with_work_graph(
            goal,
            compile_commands,
            test_commands,
            verify_commands,
            expected_files,
            WorkGraphRequest::default(),
        )
        .await
    }

    /// Begin a node and persist the bounded, code-selected Work-Graph that
    /// governs its subsequent routing.
    pub async fn begin_node_with_work_graph(
        &self,
        goal: String,
        compile_commands: Vec<String>,
        test_commands: Vec<String>,
        verify_commands: Vec<String>,
        expected_files: Vec<String>,
        graph_request: WorkGraphRequest,
    ) -> Result<NodeId> {
        let project_root = {
            let coord = self
                .coordinator
                .read()
                .map_err(|e| anyhow::anyhow!("coordinator read lock: {e}"))?;
            coord.project_root().to_path_buf()
        };
        let verification_profile = VerificationProfile::detect(&project_root);
        let resolved_commands =
            verification_profile.resolve(compile_commands, test_commands, verify_commands);
        let mut coord = self
            .coordinator
            .write()
            .map_err(|e| anyhow::anyhow!("coordinator write lock: {e}"))?;

        let start_turn_id = coord
            .session()
            .current_turn_record()
            .context("begin_node requires an active turn/checkpoint")?
            .turn_id
            .clone();

        // Precondition: current node must be Verified or absent.
        if let Some(node) = coord.current_node() {
            if node.status != NodeStatus::Verified {
                anyhow::bail!(
                    "cannot begin_node: current node {:?} is not Verified",
                    node.status
                );
            }
        }

        let node_id = next_node_id(&coord)?;
        let node = Node {
            id: node_id.clone(),
            contract: NodeContract {
                goal,
                verify_commands: resolved_commands.verify_commands,
                compile_commands: resolved_commands.compile_commands,
                test_commands: resolved_commands.test_commands,
                verification_profile,
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
        coord.work_state_mut().reset_for_new_node();
        coord
            .work_state_mut()
            .set_selected_work_graph(select_work_graph(&graph_request));
        coord
            .capture_current_work_state()
            .context("persist fresh node work state")?;
        let persisted_node = coord
            .current_node()
            .context("read persisted node contract")?;
        let audit_context = WorkGraphAuditContext {
            node_id: persisted_node.id.clone(),
            attempt: 1,
        };
        let profile = project_audit_profile(persisted_node.contract.verification_profile);
        let resolved_commands = GraphAuditCommands {
            compile_commands: persisted_node.contract.compile_commands.clone(),
            test_commands: persisted_node.contract.test_commands.clone(),
            verify_commands: persisted_node.contract.verify_commands.clone(),
        };
        coord
            .work_state_mut()
            .append_graph_audit(profile_audit_event(
                &audit_context,
                profile,
                resolved_commands,
            ));
        coord
            .capture_current_work_state()
            .context("persist resolved profile audit event")?;
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
            let failure_reason = outcome_ref.fail_reason.as_ref().map(|f| format!("{f:?}"));

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
    use crate::exec_session::verify_gate::{
        CommandExecutor, CommandRun, VerifyLog, VerifyLogFinalStatus,
    };
    use crate::exec_session::{VerifyFailAction, WorkGraphStep};
    use crate::org_graph::{GraphAuditCommands, GraphAuditKind, GraphAuditRoute, NodeType};
    use crate::tools::checkpoint_store::CheckpointStore;
    use async_trait::async_trait;
    use tempfile::TempDir;

    use std::collections::VecDeque;
    use std::path::Path;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;
    use std::time::Duration;
    use tokio::sync::{Barrier, Semaphore};

    /// Mock command executor with a configurable exit code.
    struct MockExecutor {
        exit_code: i32,
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl CommandExecutor for MockExecutor {
        async fn execute(&self, command: &str, _project_root: &Path) -> Result<CommandRun> {
            self.calls.fetch_add(1, Ordering::Relaxed);
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

    /// Scripted result consumed by the test executor in command-call order.
    #[derive(Clone)]
    struct ScriptedCommandResult {
        exit_code: i32,
        stderr: String,
    }

    impl ScriptedCommandResult {
        fn success() -> Self {
            Self {
                exit_code: 0,
                stderr: String::new(),
            }
        }

        fn failure(exit_code: i32, stderr: impl Into<String>) -> Self {
            Self {
                exit_code,
                stderr: stderr.into(),
            }
        }
    }

    /// Command executor whose configured results are consumed in call order.
    /// This keeps the runtime and verification gate real while making
    /// individual anchor failures and diagnostics deterministic.
    struct ScriptedExecutor {
        results: Mutex<VecDeque<ScriptedCommandResult>>,
        calls: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl CommandExecutor for ScriptedExecutor {
        async fn execute(&self, command: &str, _project_root: &Path) -> Result<CommandRun> {
            let result = self
                .results
                .lock()
                .map_err(|e| anyhow::anyhow!("result script lock: {e}"))?
                .pop_front()
                .context("scripted executor ran out of configured results")?;
            self.calls
                .lock()
                .map_err(|e| anyhow::anyhow!("command call log lock: {e}"))?
                .push(command.to_string());
            Ok(CommandRun {
                cmd: command.to_string(),
                exit_code: Some(result.exit_code),
                stdout: String::new(),
                stderr: result.stderr,
            })
        }
    }

    struct BarrierExecutor {
        calls: Arc<Mutex<Vec<String>>>,
        call_count: AtomicUsize,
        first_call_started: Arc<Barrier>,
        release_first_call: Arc<Barrier>,
        concurrent_call: Arc<Semaphore>,
    }

    #[async_trait]
    impl CommandExecutor for BarrierExecutor {
        async fn execute(&self, command: &str, _project_root: &Path) -> Result<CommandRun> {
            let call_index = self.call_count.fetch_add(1, Ordering::SeqCst);
            self.calls
                .lock()
                .map_err(|e| anyhow::anyhow!("command call log lock: {e}"))?
                .push(command.to_string());
            if call_index == 0 {
                self.first_call_started.wait().await;
                self.release_first_call.wait().await;
            } else {
                self.concurrent_call.add_permits(1);
            }
            Ok(CommandRun {
                cmd: command.to_string(),
                exit_code: Some(0),
                stdout: String::new(),
                stderr: String::new(),
            })
        }
    }

    /// Test fixture: creates a coordinator, verify gate, and node runtime in
    /// a temp directory.
    struct TestSetup {
        runtime: NodeRuntime,
        coord: Arc<RwLock<SessionCoordinator>>,
        calls: Arc<AtomicUsize>,
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
            let calls = Arc::new(AtomicUsize::new(0));
            let executor = Arc::new(MockExecutor {
                exit_code,
                calls: Arc::clone(&calls),
            });
            let gate = Arc::new(VerifyGate::new_with_default_hooks(
                Arc::clone(&coord),
                executor,
            ));
            let runtime = NodeRuntime::new_with_default_hooks(Arc::clone(&coord), gate, 2);
            Self {
                runtime,
                coord,
                calls,
                _dir: dir,
            }
        }

        fn begin_turn(&self) {
            self.coord.write().unwrap().begin_turn().unwrap();
        }

        fn write_cargo_manifest(&self) {
            std::fs::write(
                self._dir.path().join("Cargo.toml"),
                "[package]\nname = \"verification-profile-test\"\nversion = \"0.1.0\"\n",
            )
            .expect("write Cargo.toml");
        }
    }

    struct ScriptedSetup {
        runtime: NodeRuntime,
        coord: Arc<RwLock<SessionCoordinator>>,
        calls: Arc<Mutex<Vec<String>>>,
        store: Arc<CheckpointStore>,
        _dir: TempDir,
    }

    impl ScriptedSetup {
        fn new(exit_codes: impl IntoIterator<Item = i32>) -> Self {
            Self::with_fixed_stderr(exit_codes, None)
        }

        fn with_fixed_stderr(
            exit_codes: impl IntoIterator<Item = i32>,
            fixed_stderr: Option<String>,
        ) -> Self {
            let results = exit_codes
                .into_iter()
                .map(|exit_code| ScriptedCommandResult {
                    exit_code,
                    stderr: fixed_stderr.clone().unwrap_or_else(|| {
                        if exit_code == 0 {
                            String::new()
                        } else {
                            "command failed".to_string()
                        }
                    }),
                });
            Self::with_results(results)
        }

        fn with_results(results: impl IntoIterator<Item = ScriptedCommandResult>) -> Self {
            Self::with_results_and_gate_hooks(results, Arc::new(NoHooks))
        }

        fn with_results_and_gate_hooks(
            results: impl IntoIterator<Item = ScriptedCommandResult>,
            hooks: Arc<dyn SessionHooks>,
        ) -> Self {
            let dir = TempDir::new().expect("temporary project");
            let store = Arc::new(CheckpointStore::new(dir.path()));
            let coord = SessionCoordinator::new(
                "es-scripted".into(),
                SessionSource::AgentSelf,
                dir.path(),
                Arc::clone(&store),
            )
            .expect("create coordinator");
            let coord = Arc::new(RwLock::new(coord));
            let calls = Arc::new(Mutex::new(Vec::new()));
            let executor = Arc::new(ScriptedExecutor {
                results: Mutex::new(results.into_iter().collect()),
                calls: Arc::clone(&calls),
            });
            let gate = Arc::new(VerifyGate::new(Arc::clone(&coord), executor, hooks));
            let runtime = NodeRuntime::new_with_default_hooks(Arc::clone(&coord), gate, 2);
            Self {
                runtime,
                coord,
                calls,
                store,
                _dir: dir,
            }
        }

        fn begin_turn(&self) {
            self.coord
                .write()
                .expect("coordinator")
                .begin_turn()
                .expect("begin turn");
        }

        fn write_cargo_manifest(&self) {
            std::fs::write(
                self._dir.path().join("Cargo.toml"),
                "[package]\nname = \"state-isolation-test\"\nversion = \"0.1.0\"\n",
            )
            .expect("write Cargo.toml");
        }

        fn calls(&self) -> Vec<String> {
            self.calls.lock().expect("command call log").clone()
        }

        fn verify_log(&self) -> VerifyLog {
            let session_dir = self
                .coord
                .read()
                .expect("coordinator")
                .session_dir()
                .to_path_buf();
            std::fs::read_to_string(session_dir.join("verify_log.json"))
                .ok()
                .and_then(|json| serde_json::from_str(&json).ok())
                .unwrap_or_default()
        }

        fn capture_file(&self, path: &str, contents: &str) {
            let checkpoint_turn_id = self
                .coord
                .read()
                .expect("coordinator")
                .session()
                .current_turn_record()
                .expect("active turn")
                .checkpoint_turn_id
                .clone();
            let file_path = self._dir.path().join(path);
            std::fs::create_dir_all(
                file_path
                    .parent()
                    .expect("captured file has a parent directory"),
            )
            .expect("create captured file parent");
            std::fs::write(&file_path, contents).expect("write captured file");
            self.store
                .try_capture_file(&checkpoint_turn_id, path)
                .expect("capture changed file in checkpoint manifest");
        }

        /// Simulates the separately-tested authenticated RootCause tool after
        /// an anchored failure so runtime retry tests can exercise the fixed
        /// route without embedding agent lifecycle setup in every fixture.
        fn record_root_cause_handoff(&self) {
            let mut coordinator = self.coord.write().expect("coordinator");
            coordinator
                .work_state_mut()
                .set_specialist_report(
                    NodeType::RootCause,
                    crate::org_graph::SpecialistReport {
                        producer: NodeType::RootCause,
                        kind: crate::org_graph::SpecialistReportKind::RootCause,
                        summary: "The failing branch lacks a guard.".into(),
                        evidence: vec![crate::org_graph::SpecialistEvidence {
                            path: "src/guard.rs".into(),
                            detail: "Validation occurs after the failing branch.".into(),
                        }],
                        suspected_files: vec!["src/guard.rs".into()],
                        recommended_actions: vec!["Move validation before the branch.".into()],
                    },
                )
                .expect("record root-cause handoff");
            coordinator
                .capture_current_work_state()
                .expect("persist root-cause handoff");
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
    async fn begin_node_persists_default_bounded_work_graph_in_checkpoint() {
        let setup = TestSetup::new(0);
        setup.begin_turn();

        setup
            .runtime
            .begin_node("implement feature".into(), vec!["echo ok".into()], vec![])
            .await
            .expect("begin node");

        let coordinator = setup.coord.read().expect("coordinator");
        let plan = coordinator
            .work_state()
            .selected_work_graph()
            .expect("selected graph persisted");
        assert_eq!(plan.template_id, "implementation-v1");
    }

    #[tokio::test]
    async fn human_review_template_pauses_then_completes_only_after_approval() {
        let setup = TestSetup::new(0);
        setup.begin_turn();
        setup
            .runtime
            .begin_node_with_work_graph(
                "reviewed implementation".into(),
                vec!["echo compile".into()],
                vec!["echo test".into()],
                vec!["echo verify".into()],
                vec![],
                WorkGraphRequest {
                    task_kind: crate::org_graph::WorkGraphTaskKind::Implementation,
                    requires_human_review: true,
                },
            )
            .await
            .expect("begin reviewed node");

        let result = setup
            .runtime
            .run_work_graph(
                vec!["echo compile".into()],
                vec!["echo test".into()],
                vec!["echo verify".into()],
                vec![],
            )
            .await
            .expect("run anchors");
        assert_eq!(result.next_step, WorkGraphStep::AwaitHumanReview);

        let result = setup
            .runtime
            .record_human_review(HumanReview::Approve)
            .expect("approve review");
        assert_eq!(result.next_step, WorkGraphStep::Complete);
        assert_eq!(
            setup
                .coord
                .read()
                .expect("coordinator")
                .current_node()
                .expect("node")
                .status,
            NodeStatus::Verified
        );
    }

    #[tokio::test]
    async fn begin_node_without_active_turn_fails_without_mutating_node_or_audit() {
        let setup = TestSetup::new(0);

        let error = setup
            .runtime
            .begin_node("test goal".into(), vec!["echo ok".into()], vec![])
            .await
            .expect_err("begin_node must require an active turn");

        assert!(format!("{error:#}").contains("active turn"));
        assert_eq!(setup.calls.load(Ordering::Relaxed), 0);
        let coord = setup.coord.read().expect("coordinator");
        assert!(coord.node_states().is_empty());
        assert!(coord.current_node().is_none());
        assert!(coord.work_state().graph_audit().is_empty());
    }

    #[tokio::test]
    async fn work_graph_without_active_turn_fails_without_mutating_node_or_audit() {
        let setup = TestSetup::new(0);
        let node = Node {
            id: "n1".into(),
            contract: NodeContract {
                goal: "persisted without a turn".into(),
                verify_commands: vec![],
                compile_commands: vec!["cargo check".into()],
                test_commands: vec![],
                verification_profile: VerificationProfile::Rust,
                expected_files: vec![],
            },
            status: NodeStatus::Running,
            start_turn_id: "missing-turn".into(),
            retry_count: 0,
            verify_log_path: String::new(),
            created_at: chrono::Utc::now().to_rfc3339(),
        };
        setup
            .coord
            .write()
            .expect("coordinator")
            .add_node(node.clone())
            .expect("seed persisted node");

        let error = setup
            .runtime
            .run_work_graph(vec!["cargo check".into()], vec![], vec![], vec![])
            .await
            .expect_err("audited graph must require an active turn");

        assert!(format!("{error:#}").contains("active turn"));
        assert_eq!(setup.calls.load(Ordering::Relaxed), 0);
        let coord = setup.coord.read().expect("coordinator");
        assert_eq!(coord.node_states(), [node]);
        assert!(coord.work_state().graph_audit().is_empty());
    }

    #[tokio::test]
    async fn begin_node_in_rust_project_persists_code_owned_profile_commands() {
        let setup = TestSetup::new(0);
        setup.write_cargo_manifest();
        setup.begin_turn();

        setup
            .runtime
            .begin_node_with_anchors(
                "goal".into(),
                vec![],
                vec![],
                vec!["cargo test --doc".into()],
                vec![],
            )
            .await
            .expect("begin node");

        let coord = setup.coord.read().expect("coordinator");
        let contract = &coord.current_node().expect("node").contract;
        assert_eq!(contract.verification_profile, VerificationProfile::Rust);
        assert_eq!(contract.compile_commands, ["cargo check"]);
        assert_eq!(contract.test_commands, ["cargo test --all"]);
        assert_eq!(
            contract.verify_commands,
            [
                "cargo clippy --all-targets -- -D warnings",
                "cargo test --doc",
            ]
        );
    }

    #[tokio::test]
    async fn begin_node_in_non_rust_project_preserves_declared_anchors() {
        let setup = TestSetup::new(0);
        setup.begin_turn();

        setup
            .runtime
            .begin_node_with_anchors(
                "goal".into(),
                vec!["compile".into(), "compile".into()],
                vec!["test".into(), "test".into()],
                vec!["verify".into(), "verify".into()],
                vec![],
            )
            .await
            .expect("begin node");

        let coord = setup.coord.read().expect("coordinator");
        let contract = &coord.current_node().expect("node").contract;
        assert_eq!(contract.verification_profile, VerificationProfile::None);
        assert_eq!(contract.compile_commands, ["compile", "compile"]);
        assert_eq!(contract.test_commands, ["test", "test"]);
        assert_eq!(contract.verify_commands, ["verify", "verify"]);
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
    async fn compile_failure_is_persisted_and_does_not_run_test_anchor() {
        let setup = TestSetup::new(1);
        setup.begin_turn();
        setup
            .runtime
            .begin_node_with_anchors(
                "goal".into(),
                vec!["cargo check".into()],
                vec!["cargo test".into()],
                vec!["cargo test --doc".into()],
                vec![],
            )
            .await
            .expect("persist work-graph node");

        let result = setup
            .runtime
            .run_work_graph(
                vec!["cargo check".into()],
                vec!["cargo test".into()],
                vec!["cargo test --doc".into()],
                vec![],
            )
            .await
            .expect("run work graph");

        assert_eq!(result.next_step, WorkGraphStep::RootCause);
        assert_eq!(setup.calls.load(Ordering::Relaxed), 1);
        let coord = setup.coord.read().expect("coordinator read lock");
        assert!(coord
            .work_state()
            .compile_result(NodeType::Verification)
            .expect("verification may read compile result")
            .is_some());
    }

    #[tokio::test]
    async fn run_work_graph_without_persisted_node_returns_actionable_error() {
        let setup = TestSetup::new(0);
        setup.begin_turn();

        let error = setup
            .runtime
            .run_work_graph(
                vec!["compile".into()],
                vec!["test".into()],
                vec!["verify".into()],
                vec![],
            )
            .await
            .expect_err("work graph must require a persisted current node");

        assert!(error.to_string().contains("persisted current node"));
        assert_eq!(setup.calls.load(Ordering::Relaxed), 0);
        assert!(setup
            .coord
            .read()
            .expect("coordinator")
            .work_state()
            .graph_audit()
            .is_empty());
    }

    #[tokio::test]
    async fn concurrent_work_graph_passes_are_serialized_with_distinct_attempts() {
        let dir = TempDir::new().expect("temporary project");
        let store = Arc::new(CheckpointStore::new(dir.path()));
        let coord = Arc::new(RwLock::new(
            SessionCoordinator::new(
                "es-concurrent".into(),
                SessionSource::AgentSelf,
                dir.path(),
                store,
            )
            .expect("create coordinator"),
        ));
        let calls = Arc::new(Mutex::new(Vec::new()));
        let first_call_started = Arc::new(Barrier::new(2));
        let release_first_call = Arc::new(Barrier::new(2));
        let concurrent_call = Arc::new(Semaphore::new(0));
        let executor = Arc::new(BarrierExecutor {
            calls: Arc::clone(&calls),
            call_count: AtomicUsize::new(0),
            first_call_started: Arc::clone(&first_call_started),
            release_first_call: Arc::clone(&release_first_call),
            concurrent_call: Arc::clone(&concurrent_call),
        });
        let gate = Arc::new(VerifyGate::new_with_default_hooks(
            Arc::clone(&coord),
            executor,
        ));
        let runtime = NodeRuntime::new_with_default_hooks(Arc::clone(&coord), gate, 2);
        coord
            .write()
            .expect("coordinator")
            .begin_turn()
            .expect("begin turn");
        runtime
            .begin_node_with_anchors(
                "goal".into(),
                vec!["compile".into()],
                vec!["test".into()],
                vec!["verify".into()],
                vec![],
            )
            .await
            .expect("begin node");

        let commands = || {
            (
                vec!["compile".to_string()],
                vec!["test".to_string()],
                vec!["verify".to_string()],
            )
        };
        let (compile, test, verify) = commands();
        let first = runtime.run_work_graph(compile, test, verify, vec![]);
        let (compile, test, verify) = commands();
        let second = runtime.run_work_graph(compile, test, verify, vec![]);
        let observe_serialization = async {
            first_call_started.wait().await;
            assert!(
                tokio::time::timeout(Duration::from_millis(100), concurrent_call.acquire())
                    .await
                    .is_err(),
                "a second graph pass entered command execution before the first pass finished"
            );
            release_first_call.wait().await;
        };

        let (first, second, ()) = tokio::join!(first, second, observe_serialization);
        assert_eq!(
            first.expect("first pass").next_step,
            WorkGraphStep::Complete
        );
        assert_eq!(
            second.expect("second pass").next_step,
            WorkGraphStep::Complete
        );
        assert_eq!(
            calls.lock().expect("command call log").as_slice(),
            ["compile", "test", "verify", "compile", "test", "verify"]
        );
        let attempts: Vec<_> = coord
            .read()
            .expect("coordinator")
            .work_state()
            .graph_audit()
            .iter()
            .filter(|event| event.kind != GraphAuditKind::ProfileResolved)
            .map(|event| event.attempt)
            .collect();
        assert_eq!(attempts, [1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 2, 2]);
    }

    #[tokio::test]
    async fn passing_anchors_complete_the_fixed_work_graph() {
        let setup = TestSetup::new(0);
        setup.begin_turn();
        setup
            .runtime
            .begin_node_with_anchors(
                "goal".into(),
                vec!["cargo check".into()],
                vec!["cargo test".into()],
                vec!["cargo test --doc".into()],
                vec![],
            )
            .await
            .expect("persist work-graph node");

        let result = setup
            .runtime
            .run_work_graph(
                vec!["cargo check".into()],
                vec!["cargo test".into()],
                vec!["cargo test --doc".into()],
                vec![],
            )
            .await
            .expect("run work graph");

        assert_eq!(result.next_step, WorkGraphStep::Complete);
        assert_eq!(setup.calls.load(Ordering::Relaxed), 3);
        let coord = setup.coord.read().expect("coordinator read lock");
        assert!(
            coord
                .work_state()
                .compile_result(NodeType::Verification)
                .expect("verification may read compile result")
                .expect("compile result present")
                .ok
        );
        assert!(
            coord
                .work_state()
                .test_result(NodeType::Verification)
                .expect("verification may read test result")
                .expect("test result present")
                .pass
        );
        assert!(
            coord
                .work_state()
                .verify_result(NodeType::Verification)
                .expect("verification may read verify result")
                .expect("verify result present")
                .success
        );
        assert!(coord
            .work_state()
            .generated_diff(NodeType::GeneralPurpose)
            .expect("work node may read generated diff")
            .is_some());
    }

    #[tokio::test]
    async fn rust_work_graph_persists_real_anchor_and_route_audit_sequence() {
        let setup = ScriptedSetup::new([0, 0, 0]);
        setup.write_cargo_manifest();
        setup.begin_turn();
        setup
            .runtime
            .begin_node("goal".into(), vec![], vec![])
            .await
            .expect("begin rust node");
        setup
            .coord
            .write()
            .expect("coordinator")
            .work_state_mut()
            .set_budget(
                NodeType::GeneralPurpose,
                Budget {
                    max_iter: 7,
                    iter_used: 3,
                    token_used: 42,
                },
            )
            .expect("set state-owned budget");

        assert!(matches!(
            setup
                .runtime
                .verify_current_node()
                .await
                .expect("verify rust node"),
            NodeVerificationOutcome::WorkGraph(_)
        ));

        let coord = setup.coord.read().expect("coordinator");
        let audit = coord.work_state().graph_audit();
        assert_eq!(
            audit.iter().map(|event| &event.kind).collect::<Vec<_>>(),
            [
                &GraphAuditKind::ProfileResolved,
                &GraphAuditKind::AnchorCompleted,
                &GraphAuditKind::RouteSelected,
                &GraphAuditKind::AnchorCompleted,
                &GraphAuditKind::RouteSelected,
                &GraphAuditKind::AnchorCompleted,
                &GraphAuditKind::RouteSelected,
            ]
        );
        assert_eq!(audit[0].node_id, "n1");
        assert_eq!(audit[0].attempt, 1);
        assert_eq!(audit[0].profile, Some(GraphAuditProfile::Rust));
        assert_eq!(
            audit[0].resolved_commands,
            Some(GraphAuditCommands {
                compile_commands: vec!["cargo check".into()],
                test_commands: vec!["cargo test --all".into()],
                verify_commands: vec!["cargo clippy --all-targets -- -D warnings".into()],
            })
        );
        assert_eq!(audit[1].anchor, Some(GraphAuditAnchor::Compile));
        assert_eq!(audit[1].commands[0].command, "cargo check");
        assert_eq!(audit[1].commands[0].exit_code, Some(0));
        assert!(audit
            .iter()
            .filter(|event| event.kind == GraphAuditKind::RouteSelected)
            .all(|event| {
                event.budget
                    == Some(Budget {
                        max_iter: 7,
                        iter_used: 3,
                        token_used: 42,
                    })
            }));
        assert_eq!(
            audit.last().expect("final route event").route,
            Some(GraphAuditRoute::Complete)
        );
        assert_eq!(
            coord.current_node().expect("node").status,
            NodeStatus::Verified
        );
        assert_eq!(coord.session().status, SessionStatus::InProgress);
        drop(coord);
        assert_eq!(
            setup.verify_log().final_status,
            Some(VerifyLogFinalStatus::Completed)
        );
    }

    #[tokio::test]
    async fn audit_compile_failure_records_root_cause_route_and_consumed_budget() {
        let setup = ScriptedSetup::with_results([ScriptedCommandResult::failure(
            17,
            "compile diagnostics",
        )]);
        setup.write_cargo_manifest();
        setup.begin_turn();
        setup
            .runtime
            .begin_node("goal".into(), vec![], vec![])
            .await
            .expect("persist rust node before work graph invocation");
        let outcome = setup
            .runtime
            .verify_current_node()
            .await
            .expect("run failed compile pass");

        assert!(matches!(
            outcome,
            NodeVerificationOutcome::WorkGraph(WorkGraphRunResult {
                next_step: WorkGraphStep::RootCause
            })
        ));
        assert_eq!(setup.calls(), ["cargo check"]);
        let coord = setup.coord.read().expect("coordinator");
        let audit = coord.work_state().graph_audit();
        let route = audit.last().expect("compile route audit");
        assert_eq!(route.route, Some(GraphAuditRoute::RootCause));
        assert_eq!(
            route.budget,
            Some(Budget {
                max_iter: 2,
                iter_used: 1,
                token_used: 0,
            })
        );
        assert_eq!(coord.session().status, SessionStatus::InProgress);
        drop(coord);
        assert_eq!(setup.verify_log().final_status, None);
    }

    #[tokio::test]
    async fn audit_test_failure_retry_restarts_compile_and_retains_attempts() {
        let setup = ScriptedSetup::with_results([
            ScriptedCommandResult::success(),
            ScriptedCommandResult::failure(1, "test diagnostics"),
            ScriptedCommandResult::success(),
            ScriptedCommandResult::success(),
            ScriptedCommandResult::success(),
        ]);
        setup.write_cargo_manifest();
        setup.begin_turn();
        setup
            .runtime
            .begin_node("goal".into(), vec![], vec![])
            .await
            .expect("persist rust node before work graph invocation");

        let first = setup
            .runtime
            .verify_current_node()
            .await
            .expect("run failed test pass");
        {
            let coord = setup.coord.read().expect("coordinator");
            assert_eq!(coord.session().status, SessionStatus::InProgress);
        }
        assert_eq!(setup.verify_log().final_status, None);
        setup.record_root_cause_handoff();
        let retry = setup
            .runtime
            .verify_current_node()
            .await
            .expect("run complete retry pass");

        assert!(matches!(
            first,
            NodeVerificationOutcome::WorkGraph(WorkGraphRunResult {
                next_step: WorkGraphStep::RootCause
            })
        ));
        assert!(matches!(
            retry,
            NodeVerificationOutcome::WorkGraph(WorkGraphRunResult {
                next_step: WorkGraphStep::Complete
            })
        ));
        assert_eq!(
            setup.calls(),
            [
                "cargo check",
                "cargo test --all",
                "cargo check",
                "cargo test --all",
                "cargo clippy --all-targets -- -D warnings",
            ]
        );
        let coord = setup.coord.read().expect("coordinator");
        let audit = coord.work_state().graph_audit();
        let route_events: Vec<_> = audit
            .iter()
            .filter(|event| event.kind == GraphAuditKind::RouteSelected)
            .collect();
        assert_eq!(
            route_events
                .iter()
                .map(|event| (event.attempt, event.route, event.budget.clone()))
                .collect::<Vec<_>>(),
            [
                (
                    1,
                    Some(GraphAuditRoute::TestAnchor),
                    Some(Budget {
                        max_iter: 2,
                        iter_used: 0,
                        token_used: 0,
                    }),
                ),
                (
                    1,
                    Some(GraphAuditRoute::RootCause),
                    Some(Budget {
                        max_iter: 2,
                        iter_used: 1,
                        token_used: 0,
                    }),
                ),
                (
                    2,
                    Some(GraphAuditRoute::TestAnchor),
                    Some(Budget {
                        max_iter: 2,
                        iter_used: 1,
                        token_used: 0,
                    }),
                ),
                (
                    2,
                    Some(GraphAuditRoute::VerifyGate),
                    Some(Budget {
                        max_iter: 2,
                        iter_used: 1,
                        token_used: 0,
                    }),
                ),
                (
                    2,
                    Some(GraphAuditRoute::Complete),
                    Some(Budget {
                        max_iter: 2,
                        iter_used: 1,
                        token_used: 0,
                    }),
                ),
            ]
        );
    }

    #[tokio::test]
    async fn audit_boundary_violation_records_escalate_route() {
        let setup = ScriptedSetup::with_results([
            ScriptedCommandResult::success(),
            ScriptedCommandResult::success(),
            ScriptedCommandResult::success(),
        ]);
        setup.write_cargo_manifest();
        setup.begin_turn();
        setup
            .runtime
            .begin_node("goal".into(), vec![], vec![])
            .await
            .expect("persist rust node before work graph invocation");
        setup.capture_file("src/unexpected.rs", "unexpected change\n");

        let outcome = setup
            .runtime
            .verify_current_node()
            .await
            .expect("run boundary violation pass");

        assert!(matches!(
            outcome,
            NodeVerificationOutcome::WorkGraph(WorkGraphRunResult {
                next_step: WorkGraphStep::Escalate
            })
        ));
        assert_eq!(
            setup.calls(),
            [
                "cargo check",
                "cargo test --all",
                "cargo clippy --all-targets -- -D warnings",
            ]
        );
        let coord = setup.coord.read().expect("coordinator");
        let audit = coord.work_state().graph_audit();
        let route = audit.last().expect("verification route audit");
        assert_eq!(route.route, Some(GraphAuditRoute::Escalate));
        assert_eq!(
            route.budget,
            Some(Budget {
                max_iter: 2,
                iter_used: 0,
                token_used: 0,
            })
        );
        assert_eq!(coord.session().status, SessionStatus::Failed);
        assert_eq!(
            coord.current_node().expect("current node").status,
            NodeStatus::Failed
        );
        drop(coord);
        assert_eq!(
            setup.verify_log().final_status,
            Some(VerifyLogFinalStatus::Failed)
        );
    }

    #[tokio::test]
    async fn audit_exhausted_final_verification_records_consumed_budget_and_escalates_session() {
        let setup = ScriptedSetup::with_results([
            ScriptedCommandResult::success(),
            ScriptedCommandResult::success(),
            ScriptedCommandResult::failure(9, "final verification diagnostics"),
        ]);
        setup.write_cargo_manifest();
        setup.begin_turn();
        setup
            .runtime
            .begin_node("goal".into(), vec![], vec![])
            .await
            .expect("persist rust node before work graph invocation");
        setup
            .coord
            .write()
            .expect("coordinator")
            .work_state_mut()
            .set_budget(
                NodeType::GeneralPurpose,
                Budget {
                    max_iter: 1,
                    iter_used: 0,
                    token_used: 0,
                },
            )
            .expect("set exhausted-after-failure budget");

        let outcome = setup
            .runtime
            .verify_current_node()
            .await
            .expect("run exhausted final verification pass");

        assert!(matches!(
            outcome,
            NodeVerificationOutcome::WorkGraph(WorkGraphRunResult {
                next_step: WorkGraphStep::Escalate
            })
        ));
        assert_eq!(
            setup.calls(),
            [
                "cargo check",
                "cargo test --all",
                "cargo clippy --all-targets -- -D warnings",
            ]
        );
        let coord = setup.coord.read().expect("coordinator");
        let route = coord
            .work_state()
            .graph_audit()
            .last()
            .expect("verification route audit");
        assert_eq!(route.route, Some(GraphAuditRoute::Escalate));
        assert_eq!(
            route.budget,
            Some(Budget {
                max_iter: 1,
                iter_used: 1,
                token_used: 0,
            })
        );
        assert_eq!(coord.session().status, SessionStatus::Failed);
        assert_eq!(
            coord.current_node().expect("current node").status,
            NodeStatus::Failed
        );
        drop(coord);
        assert_eq!(
            setup.verify_log().final_status,
            Some(VerifyLogFinalStatus::Failed)
        );
    }

    #[tokio::test]
    async fn audit_final_verification_retry_keeps_session_and_verify_log_open() {
        let setup = ScriptedSetup::with_results([
            ScriptedCommandResult::success(),
            ScriptedCommandResult::success(),
            ScriptedCommandResult::failure(9, "final verification diagnostics"),
        ]);
        setup.write_cargo_manifest();
        setup.begin_turn();
        setup
            .runtime
            .begin_node("goal".into(), vec![], vec![])
            .await
            .expect("persist rust node before work graph invocation");

        let outcome = setup
            .runtime
            .verify_current_node()
            .await
            .expect("run retryable final verification pass");

        assert!(matches!(
            outcome,
            NodeVerificationOutcome::WorkGraph(WorkGraphRunResult {
                next_step: WorkGraphStep::RootCause
            })
        ));
        let coord = setup.coord.read().expect("coordinator");
        assert_eq!(coord.session().status, SessionStatus::InProgress);
        drop(coord);
        assert_eq!(setup.verify_log().final_status, None);
    }

    struct EscalatingGateHooks {
        calls: Arc<AtomicUsize>,
    }

    impl SessionHooks for EscalatingGateHooks {
        fn verify_fail(&self, _ctx: &crate::exec_session::VerifyFailContext) -> VerifyFailAction {
            self.calls.fetch_add(1, Ordering::SeqCst);
            VerifyFailAction::Escalate
        }
    }

    #[tokio::test]
    async fn graph_final_failure_uses_work_state_budget_without_invoking_gate_hook() {
        let hook_calls = Arc::new(AtomicUsize::new(0));
        let setup = ScriptedSetup::with_results_and_gate_hooks(
            [
                ScriptedCommandResult::success(),
                ScriptedCommandResult::success(),
                ScriptedCommandResult::failure(9, "final verification diagnostics"),
            ],
            Arc::new(EscalatingGateHooks {
                calls: Arc::clone(&hook_calls),
            }),
        );
        setup.write_cargo_manifest();
        setup.begin_turn();
        setup
            .runtime
            .begin_node("goal".into(), vec![], vec![])
            .await
            .expect("persist rust node before work graph invocation");
        let stale_log_path = setup
            .coord
            .read()
            .expect("coordinator")
            .session_dir()
            .join("verify_log.json");
        std::fs::write(
            stale_log_path,
            serde_json::to_string_pretty(&VerifyLog {
                attempts: Vec::new(),
                final_status: Some(VerifyLogFinalStatus::Completed),
            })
            .expect("serialize stale verify log"),
        )
        .expect("seed stale completed verify log");

        let outcome = setup
            .runtime
            .verify_current_node()
            .await
            .expect("run retryable final verification pass");

        assert!(matches!(
            outcome,
            NodeVerificationOutcome::WorkGraph(WorkGraphRunResult {
                next_step: WorkGraphStep::RootCause
            })
        ));
        assert_eq!(hook_calls.load(Ordering::SeqCst), 0);
        let coord = setup.coord.read().expect("coordinator");
        assert_eq!(coord.session().status, SessionStatus::InProgress);
        drop(coord);
        assert_eq!(setup.verify_log().final_status, None);
    }

    #[tokio::test]
    async fn audit_exhausted_compile_budget_records_escalate_route() {
        let setup = ScriptedSetup::with_results([ScriptedCommandResult::failure(
            101,
            "compile diagnostics",
        )]);
        setup.write_cargo_manifest();
        setup.begin_turn();
        setup
            .runtime
            .begin_node("goal".into(), vec![], vec![])
            .await
            .expect("persist rust node before work graph invocation");
        setup
            .coord
            .write()
            .expect("coordinator")
            .work_state_mut()
            .set_budget(
                NodeType::GeneralPurpose,
                Budget {
                    max_iter: 1,
                    iter_used: 0,
                    token_used: 0,
                },
            )
            .expect("set exhausted-after-failure budget");

        let outcome = setup
            .runtime
            .verify_current_node()
            .await
            .expect("run exhausted compile pass");

        assert!(matches!(
            outcome,
            NodeVerificationOutcome::WorkGraph(WorkGraphRunResult {
                next_step: WorkGraphStep::Escalate
            })
        ));
        assert_eq!(setup.calls(), ["cargo check"]);
        let coord = setup.coord.read().expect("coordinator");
        let audit = coord.work_state().graph_audit();
        let route = audit.last().expect("compile route audit");
        assert_eq!(route.route, Some(GraphAuditRoute::Escalate));
        assert_eq!(
            route.budget,
            Some(Budget {
                max_iter: 1,
                iter_used: 1,
                token_used: 0,
            })
        );
        assert_eq!(coord.session().status, SessionStatus::Failed);
        assert_eq!(
            coord.current_node().expect("current node").status,
            NodeStatus::Failed
        );
        drop(coord);
        assert_eq!(
            setup.verify_log().final_status,
            Some(VerifyLogFinalStatus::Failed)
        );
    }

    #[tokio::test]
    async fn audit_exhausted_test_budget_records_terminal_failure() {
        let setup = ScriptedSetup::with_results([
            ScriptedCommandResult::success(),
            ScriptedCommandResult::failure(1, "test diagnostics"),
        ]);
        setup.write_cargo_manifest();
        setup.begin_turn();
        setup
            .runtime
            .begin_node("goal".into(), vec![], vec![])
            .await
            .expect("persist rust node before work graph invocation");
        setup
            .coord
            .write()
            .expect("coordinator")
            .work_state_mut()
            .set_budget(
                NodeType::GeneralPurpose,
                Budget {
                    max_iter: 1,
                    iter_used: 0,
                    token_used: 0,
                },
            )
            .expect("set exhausted-after-failure budget");

        let outcome = setup
            .runtime
            .verify_current_node()
            .await
            .expect("run exhausted test pass");

        assert!(matches!(
            outcome,
            NodeVerificationOutcome::WorkGraph(WorkGraphRunResult {
                next_step: WorkGraphStep::Escalate
            })
        ));
        assert_eq!(setup.calls(), ["cargo check", "cargo test --all"]);
        let coord = setup.coord.read().expect("coordinator");
        let route = coord
            .work_state()
            .graph_audit()
            .last()
            .expect("test route audit");
        assert_eq!(route.route, Some(GraphAuditRoute::Escalate));
        assert_eq!(
            route.budget,
            Some(Budget {
                max_iter: 1,
                iter_used: 1,
                token_used: 0,
            })
        );
        assert_eq!(coord.session().status, SessionStatus::Failed);
        assert_eq!(
            coord.current_node().expect("current node").status,
            NodeStatus::Failed
        );
        drop(coord);
        assert_eq!(
            setup.verify_log().final_status,
            Some(VerifyLogFinalStatus::Failed)
        );
    }

    #[tokio::test]
    async fn audit_stderr_truncation_is_utf8_safe_and_bounded() {
        let oversized_stderr = "界".repeat(3_000);
        let setup = ScriptedSetup::with_fixed_stderr([17], Some(oversized_stderr));
        setup.write_cargo_manifest();
        setup.begin_turn();
        setup
            .runtime
            .begin_node("goal".into(), vec![], vec![])
            .await
            .expect("begin rust node");

        setup
            .runtime
            .verify_current_node()
            .await
            .expect("record failed compile anchor");

        let coord = setup.coord.read().expect("coordinator");
        let command = &coord.work_state().graph_audit()[1].commands[0];
        assert_eq!(command.command, "cargo check");
        assert_eq!(command.exit_code, Some(17));
        assert!(command.stderr.len() <= 8_192);
        assert!(std::str::from_utf8(command.stderr.as_bytes()).is_ok());
        assert!(command.stderr.ends_with("\n...[stderr truncated]"));
    }

    #[tokio::test]
    async fn graph_audit_recovers_from_checkpoint_with_event_facts_intact() {
        let setup = ScriptedSetup::new([0, 0, 0]);
        setup.write_cargo_manifest();
        setup.begin_turn();
        setup
            .runtime
            .begin_node("goal".into(), vec![], vec![])
            .await
            .expect("begin rust node");
        setup
            .runtime
            .verify_current_node()
            .await
            .expect("verify rust node");

        let persisted_checkpoint_turn_id = setup
            .coord
            .read()
            .expect("coordinator")
            .session()
            .current_turn_record()
            .expect("active turn")
            .checkpoint_turn_id
            .clone();
        let store = Arc::new(CheckpointStore::new(setup._dir.path()));
        let mut reloaded = SessionCoordinator::new(
            "es-checkpoint-reload".into(),
            SessionSource::AgentSelf,
            setup._dir.path(),
            store,
        )
        .expect("create fresh coordinator over checkpoint store");
        let (reload_turn_id, reload_checkpoint_turn_id) = {
            let turn = reloaded.begin_turn().expect("begin reload turn");
            (turn.turn_id.clone(), turn.checkpoint_turn_id.clone())
        };
        let checkpoint_root = setup._dir.path().join(".wgenty-code").join("checkpoints");
        std::fs::copy(
            checkpoint_root
                .join(persisted_checkpoint_turn_id)
                .join("work_state.json"),
            checkpoint_root
                .join(reload_checkpoint_turn_id)
                .join("work_state.json"),
        )
        .expect("copy persisted audit into fresh coordinator checkpoint");
        reloaded
            .restore_work_state_for_turn(&reload_turn_id)
            .expect("reload audit through coordinator API");
        let audit = reloaded.work_state().graph_audit();

        assert_eq!(audit[1].node_id, "n1");
        assert_eq!(audit[1].attempt, 1);
        assert_eq!(audit[1].commands[0].exit_code, Some(0));
        assert_eq!(
            audit.last().expect("final route event").route,
            Some(GraphAuditRoute::Complete)
        );
    }

    #[tokio::test]
    async fn retry_after_test_failure_runs_all_anchors_from_compile_again() {
        let setup = ScriptedSetup::new([0, 1, 0, 0, 0]);
        setup.begin_turn();
        let compile = vec!["compile".to_string()];
        let test = vec!["test".to_string()];
        let verify = vec!["verify".to_string()];
        setup
            .runtime
            .begin_node_with_anchors(
                "goal".into(),
                compile.clone(),
                test.clone(),
                verify.clone(),
                vec![],
            )
            .await
            .expect("persist work-graph node");

        let first = setup
            .runtime
            .run_work_graph(compile.clone(), test.clone(), verify.clone(), vec![])
            .await
            .expect("first work-graph pass");
        assert_eq!(first.next_step, WorkGraphStep::RootCause);
        setup.record_root_cause_handoff();

        let retry = setup
            .runtime
            .run_work_graph(compile, test, verify, vec![])
            .await
            .expect("retry work-graph pass");

        assert_eq!(retry.next_step, WorkGraphStep::Complete);
        assert_eq!(
            setup.calls(),
            ["compile", "test", "compile", "test", "verify"]
        );
    }

    #[tokio::test]
    async fn two_rust_nodes_in_one_turn_each_run_the_complete_anchor_chain() {
        let setup = ScriptedSetup::new([0, 0, 0, 0, 0, 0]);
        setup.write_cargo_manifest();
        setup.begin_turn();

        setup
            .runtime
            .begin_node("first".into(), vec![], vec![])
            .await
            .expect("begin first node");
        let first = setup
            .runtime
            .verify_current_node()
            .await
            .expect("verify first node");
        assert!(matches!(
            first,
            NodeVerificationOutcome::WorkGraph(WorkGraphRunResult {
                next_step: WorkGraphStep::Complete
            })
        ));

        setup
            .runtime
            .begin_node("second".into(), vec![], vec![])
            .await
            .expect("begin second node in the same turn");
        {
            let coord = setup.coord.read().expect("coordinator");
            let state = coord.work_state();
            assert!(state
                .compile_result(NodeType::Verification)
                .expect("read compile result")
                .is_none());
            assert!(state
                .test_result(NodeType::Verification)
                .expect("read test result")
                .is_none());
            assert!(state
                .verify_result(NodeType::Verification)
                .expect("read verify result")
                .is_none());
            assert!(state
                .generated_diff(NodeType::GeneralPurpose)
                .expect("read generated diff")
                .is_none());
            assert!(state
                .budget(NodeType::GeneralPurpose)
                .expect("read retry budget")
                .is_none());
        }
        let second = setup
            .runtime
            .verify_current_node()
            .await
            .expect("verify second node");

        assert!(matches!(
            second,
            NodeVerificationOutcome::WorkGraph(WorkGraphRunResult {
                next_step: WorkGraphStep::Complete
            })
        ));
        assert_eq!(
            setup.calls(),
            [
                "cargo check",
                "cargo test --all",
                "cargo clippy --all-targets -- -D warnings",
                "cargo check",
                "cargo test --all",
                "cargo clippy --all-targets -- -D warnings",
            ]
        );
    }

    #[tokio::test]
    async fn retry_after_restoring_failed_verify_checkpoint_reruns_every_anchor() {
        let setup = ScriptedSetup::new([0, 0, 1, 0, 0, 0]);
        setup.begin_turn();
        let compile = vec!["compile".to_string()];
        let test = vec!["test".to_string()];
        let verify = vec!["verify".to_string()];
        setup
            .runtime
            .begin_node_with_anchors(
                "goal".into(),
                compile.clone(),
                test.clone(),
                verify.clone(),
                vec![],
            )
            .await
            .expect("persist work-graph node");

        let first = setup
            .runtime
            .run_work_graph(compile.clone(), test.clone(), verify.clone(), vec![])
            .await
            .expect("failed verification pass");
        assert_eq!(first.next_step, WorkGraphStep::RootCause);
        setup.record_root_cause_handoff();

        let turn_id = {
            let mut coord = setup.coord.write().expect("coordinator");
            let turn_id = coord.current_turn_id().expect("active turn").to_string();
            *coord.work_state_mut() = crate::org_graph::WorkState::default();
            coord
                .restore_work_state_for_turn(&turn_id)
                .expect("restore persisted work state");
            assert!(
                !coord
                    .work_state()
                    .verify_result(NodeType::Verification)
                    .expect("read verify result")
                    .expect("checkpoint contains failed verify result")
                    .success
            );
            turn_id
        };

        let retry = setup
            .runtime
            .run_work_graph(compile, test, verify, vec![])
            .await
            .expect("retry after checkpoint restore");

        assert_eq!(retry.next_step, WorkGraphStep::Complete, "turn {turn_id}");
        assert_eq!(
            setup.calls(),
            ["compile", "test", "verify", "compile", "test", "verify"]
        );
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
    async fn rollback_replacement_node_uses_fresh_audit_identity_and_attempt() {
        let setup = TestSetup::new(0);
        setup.write_cargo_manifest();
        setup.begin_turn();
        setup
            .runtime
            .begin_node("node1".into(), vec![], vec![])
            .await
            .expect("begin first node");
        setup
            .runtime
            .verify_current_node()
            .await
            .expect("verify first node");

        setup.begin_turn();
        let removed_node_id = setup
            .runtime
            .begin_node("node2".into(), vec![], vec![])
            .await
            .expect("begin node that will be rolled back");
        setup
            .runtime
            .run_work_graph(
                vec!["cargo check".into()],
                vec!["cargo test --all".into()],
                vec!["cargo clippy --all-targets -- -D warnings".into()],
                vec![],
            )
            .await
            .expect("record historical audit for removed node");
        let rollback = setup.runtime.rollback_node().await.expect("rollback node2");
        assert_eq!(rollback.removed_nodes, vec![removed_node_id.clone()]);
        assert!(setup
            .coord
            .read()
            .expect("coordinator")
            .work_state()
            .graph_audit()
            .iter()
            .any(|event| event.node_id == removed_node_id));

        let replacement_id = setup
            .runtime
            .begin_node("replacement".into(), vec![], vec![])
            .await
            .expect("begin replacement node");
        setup
            .runtime
            .run_work_graph(
                vec!["cargo check".into()],
                vec!["cargo test --all".into()],
                vec!["cargo clippy --all-targets -- -D warnings".into()],
                vec![],
            )
            .await
            .expect("record replacement audit");

        assert_ne!(replacement_id, removed_node_id);
        let coord = setup.coord.read().expect("coordinator");
        let replacement_audit: Vec<_> = coord
            .work_state()
            .graph_audit()
            .iter()
            .filter(|event| event.node_id == replacement_id)
            .collect();
        assert!(!replacement_audit.is_empty());
        assert!(replacement_audit.iter().all(|event| event.attempt == 1));
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
        let on_disk: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&ws_path).unwrap()).unwrap();
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
