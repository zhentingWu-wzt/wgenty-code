//! Trusted per-agent-session ownership for Work-Graph runtimes.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};

use anyhow::{Context, Result};
use serde_json::json;

use crate::agent::SessionId;
use crate::org_graph::{
    GraphAuditEvent, GraphAuditKind, GraphAuditRoute, NodeType, SpecialistReport,
};
use crate::tools::checkpoint_store::CheckpointStore;

use super::{
    next_step, NodeRuntime, ProcessCommandExecutor, SessionCoordinator, SessionSource, VerifyGate,
    WorkGraphStep,
};

#[derive(Clone)]
struct RuntimeEntry {
    runtime: Arc<NodeRuntime>,
    coordinator: Arc<RwLock<SessionCoordinator>>,
    gate: Arc<VerifyGate>,
    pending_root_cause: Arc<Mutex<Option<PendingRootCause>>>,
}

#[derive(Clone)]
struct PendingRootCause {
    node_id: String,
    attempt: u32,
    child_id: Option<String>,
}

/// A code-derived request to run the predeclared root-cause specialist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RootCauseDispatchRequest {
    /// Diagnostic prompt assembled from trusted anchored state.
    pub prompt: String,
}

/// Result of preparing a static root-cause dispatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RootCauseDispatchState {
    /// No dispatch has been started for the current static route.
    Ready(RootCauseDispatchRequest),
    /// The route already owns a live root-cause child.
    Pending { child_id: String },
}

/// Lazily creates one isolated Work-Graph runtime for each trusted agent session.
///
/// Lookup keys are supplied by a trusted [`SessionId`], normally from a
/// [`ToolContext`](crate::agent::ToolContext). The store deliberately has no
/// input path that accepts a model-provided session identifier.
pub struct ExecutionSessionRuntimeStore {
    project_root: PathBuf,
    checkpoint_store: Arc<CheckpointStore>,
    auto_retry_max: u32,
    entries: Mutex<HashMap<SessionId, RuntimeEntry>>,
}

/// Late-bound daemon handle used by `TaskTool` to wire a reserved RootCause
/// child into the static Work-Graph before that child starts running.
pub type RootCauseRuntimeHandle = Arc<RwLock<Option<Arc<ExecutionSessionRuntimeStore>>>>;

/// Creates an initially unbound handle for daemon bootstrap.
pub fn root_cause_runtime_handle() -> RootCauseRuntimeHandle {
    Arc::new(RwLock::new(None))
}

impl ExecutionSessionRuntimeStore {
    /// Builds an empty store whose per-session runtimes use the supplied
    /// project checkpoint store and retry limit.
    pub fn new(
        project_root: PathBuf,
        checkpoint_store: Arc<CheckpointStore>,
        auto_retry_max: u32,
    ) -> Self {
        Self {
            project_root,
            checkpoint_store,
            auto_retry_max,
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// Returns the runtime belonging to `session_id`, creating it once when
    /// the trusted session first uses a Work-Graph tool.
    pub fn runtime_for(&self, session_id: &SessionId) -> Result<Arc<NodeRuntime>> {
        Ok(self.entry_for(session_id)?.runtime)
    }

    /// Ensures that `session_id` owns one active graph turn, then returns its
    /// runtime. Repeated calls leave the existing turn intact.
    pub fn ensure_turn(&self, session_id: &SessionId) -> Result<Arc<NodeRuntime>> {
        let entry = self.entry_for(session_id)?;
        let mut coordinator = entry.coordinator.write().map_err(|error| {
            anyhow::anyhow!("execution-session coordinator write lock: {error}")
        })?;
        if coordinator.current_turn_id().is_none() {
            coordinator
                .begin_turn()
                .context("start initial work-graph turn")?;
        }
        Ok(entry.runtime)
    }

    /// Returns the verification gate belonging to `session_id`, creating its
    /// scoped runtime when necessary.
    pub fn gate_for(&self, session_id: &SessionId) -> Result<Arc<VerifyGate>> {
        Ok(self.entry_for(session_id)?.gate)
    }

    /// Writes a trusted specialist handoff to the active turn and persists it
    /// beside the turn checkpoint.
    ///
    /// The caller identity comes from the tool context, never from model JSON;
    /// [`WorkState`] enforces that it may write this field and that the report
    /// producer and category match the trusted node type.
    pub fn record_specialist_report(
        &self,
        session_id: &SessionId,
        caller: NodeType,
        caller_agent_id: &str,
        report: SpecialistReport,
    ) -> Result<WorkGraphStep> {
        let entry = self.entry_for(session_id)?;
        let mut pending = entry
            .pending_root_cause
            .lock()
            .map_err(|error| anyhow::anyhow!("pending root-cause dispatch lock: {error}"))?;
        let expected_child_id = pending
            .as_ref()
            .and_then(|route| route.child_id.as_deref())
            .context("root-cause report has no dispatched specialist child")?;
        if caller != NodeType::RootCause {
            anyhow::bail!(
                "only the dispatched root-cause specialist may release the implement edge"
            );
        }
        let mut coordinator = entry.coordinator.write().map_err(|error| {
            anyhow::anyhow!("execution-session coordinator write lock: {error}")
        })?;
        if coordinator.current_turn_id().is_none() || coordinator.current_node().is_none() {
            anyhow::bail!("specialist reports require an active work-graph node and turn");
        }
        let node_id = coordinator
            .current_node()
            .context("specialist reports require a persisted current node")?
            .id
            .clone();
        let route = latest_root_cause_route(&coordinator, &node_id)?;
        let pending_route = pending
            .as_ref()
            .context("root-cause report has no pending route")?;
        if pending_route.node_id != node_id || pending_route.attempt != route.attempt {
            anyhow::bail!("root-cause report does not match the current failed graph attempt");
        }
        // `expected_child_id` is read above while holding the same pending-route
        // lock. The caller identity is checked by the ToolContext adapter before
        // this method, so this value is only a defense-in-depth audit invariant.
        if expected_child_id != caller_agent_id {
            anyhow::bail!("root-cause report caller does not match the dispatched specialist");
        }
        let budget = coordinator
            .work_state()
            .budget(NodeType::GeneralPurpose)
            .map_err(anyhow::Error::from)?
            .cloned();
        coordinator
            .work_state_mut()
            .set_specialist_report(caller, report)
            .map_err(anyhow::Error::from)
            .context("validate specialist report")?;
        let next_step = next_step(coordinator.work_state())
            .map_err(anyhow::Error::from)
            .context("route specialist report handoff")?;
        if next_step != WorkGraphStep::Implement {
            anyhow::bail!("root-cause report did not release the static implement edge");
        }
        coordinator
            .work_state_mut()
            .append_graph_audit(GraphAuditEvent {
                node_id,
                attempt: route.attempt,
                kind: GraphAuditKind::RouteSelected,
                anchor: None,
                commands: Vec::new(),
                route: Some(GraphAuditRoute::Implement),
                profile: None,
                resolved_commands: None,
                budget,
                timestamp: chrono::Utc::now().to_rfc3339(),
            });
        coordinator
            .capture_current_work_state()
            .context("persist specialist report checkpoint")?;
        *pending = None;
        Ok(next_step)
    }

    /// Reserves the current code-selected RootCause edge for one dispatcher.
    pub fn prepare_root_cause_dispatch(
        &self,
        session_id: &SessionId,
    ) -> Result<RootCauseDispatchState> {
        let entry = self.entry_for(session_id)?;
        let mut pending = entry
            .pending_root_cause
            .lock()
            .map_err(|error| anyhow::anyhow!("pending root-cause dispatch lock: {error}"))?;
        if let Some(route) = pending.as_ref() {
            if let Some(child_id) = &route.child_id {
                return Ok(RootCauseDispatchState::Pending {
                    child_id: child_id.clone(),
                });
            }
            anyhow::bail!("root-cause specialist dispatch is already being created");
        }

        let coordinator = entry
            .coordinator
            .read()
            .map_err(|error| anyhow::anyhow!("execution-session coordinator read lock: {error}"))?;
        let (request, node_id, attempt) = root_cause_dispatch_request(&coordinator)?;
        *pending = Some(PendingRootCause {
            node_id,
            attempt,
            child_id: None,
        });
        Ok(RootCauseDispatchState::Ready(request))
    }

    /// Recovers a lost RootCause child from persisted graph state without
    /// re-running anchors or restoring the child's stale identity.
    pub fn prepare_recovered_root_cause_dispatch(
        &self,
        session_id: &SessionId,
    ) -> Result<Option<RootCauseDispatchRequest>> {
        let entry = self.entry_for(session_id)?;
        let mut pending = entry
            .pending_root_cause
            .lock()
            .map_err(|error| anyhow::anyhow!("pending root-cause dispatch lock: {error}"))?;
        if pending.is_some() {
            return Ok(None);
        }

        let coordinator = entry
            .coordinator
            .read()
            .map_err(|error| anyhow::anyhow!("execution-session coordinator read lock: {error}"))?;
        if next_step(coordinator.work_state()).map_err(anyhow::Error::from)?
            != WorkGraphStep::RootCause
        {
            return Ok(None);
        }
        let (request, node_id, attempt) = root_cause_dispatch_request(&coordinator)?;
        *pending = Some(PendingRootCause {
            node_id,
            attempt,
            child_id: None,
        });
        Ok(Some(request))
    }

    /// Binds a successfully created child to the reserved RootCause route.
    pub fn bind_root_cause_child(&self, session_id: &SessionId, child_id: String) -> Result<()> {
        let entry = self.entry_for(session_id)?;
        let mut pending = entry
            .pending_root_cause
            .lock()
            .map_err(|error| anyhow::anyhow!("pending root-cause dispatch lock: {error}"))?;
        let route = pending
            .as_mut()
            .context("no root-cause route is waiting for a child")?;
        if route.child_id.is_some() {
            anyhow::bail!("root-cause route already has a dispatched child");
        }
        route.child_id = Some(child_id);
        Ok(())
    }

    /// Atomically binds `child_id` when the current static route is awaiting
    /// its first RootCause child.
    ///
    /// `TaskTool` calls this after the coordinator reserves the child but
    /// before it spawns that child's future. This closes the interval in which
    /// a fast child could publish a report before the parent knew its identity.
    /// A false result means this is an ordinary RootCause task, not the
    /// code-selected static route.
    pub fn try_bind_root_cause_child(
        &self,
        session_id: &SessionId,
        child_id: String,
    ) -> Result<bool> {
        let entry = self.entry_for(session_id)?;
        let mut pending = entry
            .pending_root_cause
            .lock()
            .map_err(|error| anyhow::anyhow!("pending root-cause dispatch lock: {error}"))?;
        let Some(route) = pending.as_mut() else {
            return Ok(false);
        };
        if route.child_id.is_some() {
            return Ok(false);
        }
        route.child_id = Some(child_id);
        Ok(true)
    }

    /// Resolves a dispatched specialist that reached a terminal lifecycle
    /// state without publishing its report.
    ///
    /// The static edge fails closed: a missing report consumes the remaining
    /// retry budget and selects `Escalate`, so the root cannot silently bypass
    /// the diagnostic gate by editing code after a failed child.
    pub fn resolve_root_cause_child_terminal(
        &self,
        session_id: &SessionId,
        child_id: &str,
    ) -> Result<()> {
        let entry = self.entry_for(session_id)?;
        let mut pending = entry
            .pending_root_cause
            .lock()
            .map_err(|error| anyhow::anyhow!("pending root-cause dispatch lock: {error}"))?;
        let Some(route) = pending.as_ref() else {
            // A successfully persisted report already consumed the route.
            return Ok(());
        };
        if route.child_id.as_deref() != Some(child_id) {
            return Ok(());
        }
        let route = route.clone();
        let mut coordinator = entry.coordinator.write().map_err(|error| {
            anyhow::anyhow!("execution-session coordinator write lock: {error}")
        })?;
        let budget = coordinator
            .work_state()
            .budget(NodeType::GeneralPurpose)
            .map_err(anyhow::Error::from)?
            .cloned()
            .map(|mut budget| {
                budget.iter_used = budget.max_iter;
                budget
            });
        if let Some(budget) = &budget {
            coordinator
                .work_state_mut()
                .set_budget(NodeType::GeneralPurpose, budget.clone())
                .map_err(anyhow::Error::from)
                .context("exhaust retry budget after root-cause child terminal")?;
        }
        coordinator
            .work_state_mut()
            .append_graph_audit(GraphAuditEvent {
                node_id: route.node_id,
                attempt: route.attempt,
                kind: GraphAuditKind::RouteSelected,
                anchor: None,
                commands: Vec::new(),
                route: Some(GraphAuditRoute::Escalate),
                profile: None,
                resolved_commands: None,
                budget,
                timestamp: chrono::Utc::now().to_rfc3339(),
            });
        coordinator
            .capture_current_work_state()
            .context("persist root-cause terminal escalation")?;
        *pending = None;
        drop(coordinator);
        self.entry_for(session_id)?
            .runtime
            .escalate_current_work_graph()
            .context("mark root-cause terminal as failed")?;
        Ok(())
    }

    /// Cancels an unbound dispatch reservation after the child launcher fails.
    pub fn cancel_root_cause_dispatch(&self, session_id: &SessionId) -> Result<()> {
        let entry = self.entry_for(session_id)?;
        let mut pending = entry
            .pending_root_cause
            .lock()
            .map_err(|error| anyhow::anyhow!("pending root-cause dispatch lock: {error}"))?;
        if pending
            .as_ref()
            .is_some_and(|route| route.child_id.is_none())
        {
            *pending = None;
        }
        Ok(())
    }

    /// Returns whether the static Work-Graph must reject root-agent mutation.
    ///
    /// A reservation is already protected before a child is bound. Once the
    /// route has no report, it remains protected; a terminal `Escalate` is
    /// instead synchronized to the durable node/session failure lifecycle so
    /// callers can observe it and start a new, explicitly scoped node.
    pub fn root_cause_pending(&self, session_id: &SessionId) -> Result<bool> {
        let entry = self.entry_for(session_id)?;
        let pending = entry
            .pending_root_cause
            .lock()
            .map_err(|error| anyhow::anyhow!("pending root-cause dispatch lock: {error}"))?;
        if pending.is_some() {
            return Ok(true);
        }
        let coordinator = entry
            .coordinator
            .read()
            .map_err(|error| anyhow::anyhow!("execution-session coordinator read lock: {error}"))?;
        Ok(matches!(
            next_step(coordinator.work_state()).map_err(anyhow::Error::from)?,
            WorkGraphStep::RootCause
        ))
    }

    fn entry_for(&self, session_id: &SessionId) -> Result<RuntimeEntry> {
        let mut entries = self
            .entries
            .lock()
            .map_err(|error| anyhow::anyhow!("execution-session runtime store lock: {error}"))?;
        if let Some(entry) = entries.get(session_id) {
            return Ok(entry.clone());
        }

        let coordinator = Arc::new(RwLock::new(
            SessionCoordinator::open_or_create(
                session_id.as_str().to_string(),
                SessionSource::AgentSelf,
                &self.project_root,
                Arc::clone(&self.checkpoint_store),
            )
            .context("open or create execution session coordinator")?,
        ));
        let gate = Arc::new(VerifyGate::new_with_default_hooks(
            Arc::clone(&coordinator),
            Arc::new(ProcessCommandExecutor),
        ));
        let entry = RuntimeEntry {
            runtime: Arc::new(NodeRuntime::new_with_default_hooks(
                Arc::clone(&coordinator),
                Arc::clone(&gate),
                self.auto_retry_max,
            )),
            coordinator,
            gate,
            pending_root_cause: Arc::new(Mutex::new(None)),
        };
        entries.insert(session_id.clone(), entry.clone());
        Ok(entry)
    }

    #[cfg(test)]
    fn turn_count_for_test(&self, session_id: &SessionId) -> usize {
        self.entries
            .lock()
            .expect("runtime store lock")
            .get(session_id)
            .expect("runtime exists")
            .coordinator
            .read()
            .expect("coordinator lock")
            .session()
            .turns
            .len()
    }

    #[cfg(test)]
    pub(crate) fn work_state_for_test(
        &self,
        session_id: &SessionId,
    ) -> crate::org_graph::WorkState {
        self.entries
            .lock()
            .expect("runtime store lock")
            .get(session_id)
            .expect("runtime exists")
            .coordinator
            .read()
            .expect("coordinator lock")
            .work_state()
            .clone()
    }

    #[cfg(test)]
    pub(crate) fn checkpointed_work_state_for_test(
        &self,
        session_id: &SessionId,
    ) -> crate::org_graph::WorkState {
        let entry = self.entry_for(session_id).expect("runtime exists");
        let coordinator = entry.coordinator.read().expect("coordinator lock");
        let turn_id = coordinator.current_turn_id().expect("active turn");
        let checkpoint_turn_id = coordinator
            .session()
            .turns
            .iter()
            .find(|turn| turn.turn_id == turn_id)
            .expect("current turn record")
            .checkpoint_turn_id
            .clone();
        drop(coordinator);
        self.checkpoint_store
            .restore_work_state(&checkpoint_turn_id)
            .expect("restore work state")
            .expect("checkpointed work state")
    }

    #[cfg(test)]
    pub(crate) fn current_node_status_for_test(&self, session_id: &SessionId) -> super::NodeStatus {
        self.entries
            .lock()
            .expect("runtime store lock")
            .get(session_id)
            .expect("runtime exists")
            .coordinator
            .read()
            .expect("coordinator lock")
            .current_node()
            .expect("current node")
            .status
            .clone()
    }

    #[cfg(test)]
    pub(crate) fn seed_root_cause_route_for_test(&self, session_id: &SessionId) {
        use crate::org_graph::{Budget, CompileResult};

        let entry = self.entry_for(session_id).expect("runtime exists");
        let mut coordinator = entry.coordinator.write().expect("coordinator lock");
        let node_id = coordinator.current_node().expect("current node").id.clone();
        coordinator
            .work_state_mut()
            .set_budget(
                NodeType::GeneralPurpose,
                Budget {
                    max_iter: 2,
                    iter_used: 1,
                    token_used: 0,
                },
            )
            .expect("seed budget");
        coordinator
            .work_state_mut()
            .set_compile_result(
                NodeType::Verification,
                CompileResult {
                    ok: false,
                    stderr: "seeded compile failure".into(),
                },
            )
            .expect("seed compile failure");
        coordinator
            .work_state_mut()
            .append_graph_audit(GraphAuditEvent {
                node_id,
                attempt: 1,
                kind: GraphAuditKind::RouteSelected,
                anchor: None,
                commands: Vec::new(),
                route: Some(GraphAuditRoute::RootCause),
                profile: None,
                resolved_commands: None,
                budget: Some(Budget {
                    max_iter: 2,
                    iter_used: 1,
                    token_used: 0,
                }),
                timestamp: chrono::Utc::now().to_rfc3339(),
            });
        coordinator
            .capture_current_work_state()
            .expect("persist seeded root-cause route");
    }
}

fn root_cause_dispatch_request(
    coordinator: &SessionCoordinator,
) -> Result<(RootCauseDispatchRequest, String, u32)> {
    let node = coordinator
        .current_node()
        .context("root-cause dispatch requires a persisted current node")?;
    let route = latest_root_cause_route(coordinator, &node.id)?;
    let state = coordinator.work_state();
    let prompt = json!({
        "node_goal": node.contract.goal,
        "attempt": route.attempt,
        "compile_result": state.compile_result(NodeType::RootCause).map_err(anyhow::Error::from)?,
        "test_result": state.test_result(NodeType::RootCause).map_err(anyhow::Error::from)?,
        "verify_result": state.verify_result(NodeType::RootCause).map_err(anyhow::Error::from)?,
    })
    .to_string();
    Ok((
        RootCauseDispatchRequest {
            prompt: format!(
                "Diagnose this code-owned failed verification attempt. Use the structured anchor evidence below; do not modify files. Submit your report with submit_specialist_report.\n\n{prompt}"
            ),
        },
        node.id.clone(),
        route.attempt,
    ))
}

fn latest_root_cause_route(
    coordinator: &SessionCoordinator,
    node_id: &str,
) -> Result<GraphAuditEvent> {
    coordinator
        .work_state()
        .graph_audit()
        .iter()
        .rev()
        .find(|event| {
            event.node_id == node_id
                && event.kind == GraphAuditKind::RouteSelected
                && event.route == Some(GraphAuditRoute::RootCause)
        })
        .cloned()
        .context("root-cause report requires the current graph route to be RootCause")
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tempfile::TempDir;

    use super::ExecutionSessionRuntimeStore;
    use crate::agent::SessionId;
    use crate::tools::checkpoint_store::CheckpointStore;

    fn test_store(dir: &TempDir) -> ExecutionSessionRuntimeStore {
        ExecutionSessionRuntimeStore::new(
            dir.path().to_path_buf(),
            Arc::new(CheckpointStore::new(dir.path())),
            2,
        )
    }

    #[test]
    fn runtime_store_reuses_one_runtime_per_session_and_isolates_other_sessions() {
        let dir = TempDir::new().expect("create tempdir");
        let store = test_store(&dir);

        let first = store
            .runtime_for(&SessionId::new("session-a"))
            .expect("create first runtime");
        let repeated = store
            .runtime_for(&SessionId::new("session-a"))
            .expect("reuse first runtime");
        let other = store
            .runtime_for(&SessionId::new("session-b"))
            .expect("create isolated runtime");

        assert!(Arc::ptr_eq(&first, &repeated));
        assert!(!Arc::ptr_eq(&first, &other));
    }

    #[test]
    fn ensure_turn_is_idempotent_for_one_session() {
        let dir = TempDir::new().expect("create tempdir");
        let store = test_store(&dir);
        let session_id = SessionId::new("session-a");

        store.ensure_turn(&session_id).expect("start graph turn");
        store
            .ensure_turn(&session_id)
            .expect("reuse active graph turn");

        assert_eq!(store.turn_count_for_test(&session_id), 1);
    }

    #[tokio::test]
    async fn recreated_store_restores_root_cause_route_without_old_child() {
        let dir = TempDir::new().expect("create tempdir");
        let session_id = SessionId::new("recover-route");
        let first = test_store(&dir);
        first.ensure_turn(&session_id).expect("start graph turn");
        first
            .runtime_for(&session_id)
            .expect("resolve runtime")
            .begin_node("diagnose".into(), Vec::new(), Vec::new())
            .await
            .expect("start graph node");
        first.seed_root_cause_route_for_test(&session_id);
        drop(first);

        let recovered = test_store(&dir);
        let request = recovered
            .prepare_recovered_root_cause_dispatch(&session_id)
            .expect("inspect recovered route")
            .expect("recover root-cause route");

        assert!(request.prompt.contains("seeded compile failure"));
        assert!(recovered
            .root_cause_pending(&session_id)
            .expect("recovered route is reserved"));
        assert!(recovered.prepare_root_cause_dispatch(&session_id).is_err());
        assert_eq!(
            recovered
                .work_state_for_test(&session_id)
                .graph_audit()
                .iter()
                .filter(|event| { event.kind == crate::org_graph::GraphAuditKind::AnchorCompleted })
                .count(),
            0,
            "recovery must not rerun anchors"
        );
    }

    #[tokio::test]
    async fn root_cause_child_terminal_escalates_and_keeps_root_fail_closed() {
        let dir = TempDir::new().expect("create tempdir");
        let store = test_store(&dir);
        let session_id = SessionId::new("terminal-root-cause-session");
        store.ensure_turn(&session_id).expect("start graph turn");
        store
            .runtime_for(&session_id)
            .expect("resolve runtime")
            .begin_node("diagnose".into(), Vec::new(), Vec::new())
            .await
            .expect("start graph node");
        store.seed_root_cause_route_for_test(&session_id);
        store
            .prepare_root_cause_dispatch(&session_id)
            .expect("reserve static route");
        assert!(store
            .try_bind_root_cause_child(&session_id, "root-cause-child".into())
            .expect("bind before spawn"));

        store
            .resolve_root_cause_child_terminal(&session_id, "root-cause-child")
            .expect("escalate terminal child");

        let state = store.work_state_for_test(&session_id);
        assert_eq!(
            crate::exec_session::next_step(&state).expect("route terminal state"),
            crate::exec_session::WorkGraphStep::Escalate
        );
        assert!(state
            .graph_audit()
            .iter()
            .any(|event| { event.route == Some(crate::org_graph::GraphAuditRoute::Escalate) }));
        assert_eq!(
            store.current_node_status_for_test(&session_id),
            crate::exec_session::NodeStatus::Failed
        );
        assert!(!store
            .root_cause_pending(&session_id)
            .expect("terminal escalation is observable, not a frozen reservation"));
    }
}
