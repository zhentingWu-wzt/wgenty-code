//! Fallback eligibility for subagent dispatch failures.
//!
//! Two interception points share this logic:
//! - Interception 1 (pre-dispatch): `CoordinatorError` from `reserve_child_in_group`
//! - Interception 2 (runtime): `ChildResult` with `error_code = subagent_model_unavailable`

use crate::agent::coordinator::{
    AgentCoordinator, ChildResult, ChildTerminalStatus, CoordinatorError,
};
use crate::agent::identity::{AgentExecutionContext, AgentId};

/// Kind of fallback to attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FallbackKind {
    /// Model endpoint failed -> swap to a backup model.
    ModelUnavailable,
    /// Structural failure (depth/concurrency/group) -> reuse parent's model.
    Structural,
}

/// Determine fallback eligibility from a pre-dispatch `CoordinatorError`.
///
/// `DepthLimitReached` / `ConcurrencyClosed` / `TaskGroup` -> `Some(Structural)`.
/// Everything else (NotVisible, ParentNotRunning, JoinFailed, Storage,
/// ChildrenStillRunning, RootHasNoTerminalState) -> `None`.
pub fn fallback_eligible_from_coordinator_error(e: &CoordinatorError) -> Option<FallbackKind> {
    match e {
        CoordinatorError::DepthLimitReached { .. } => Some(FallbackKind::Structural),
        CoordinatorError::ConcurrencyClosed => Some(FallbackKind::Structural),
        CoordinatorError::TaskGroup(_) => Some(FallbackKind::Structural),
        _ => None,
    }
}

/// Determine fallback eligibility from a runtime `ChildResult`.
///
/// `error_code = "subagent_model_unavailable"` -> `Some(ModelUnavailable)`.
/// All other codes (timeout, stuck, cancelled, generic error, tool_error,
/// parse_error, budget_exceeded) -> `None`.
pub fn fallback_eligible_from_child_result(r: &ChildResult) -> Option<FallbackKind> {
    if r.status != ChildTerminalStatus::Failed {
        return None;
    }
    match r.error_code.as_deref() {
        Some("subagent_model_unavailable") => Some(FallbackKind::ModelUnavailable),
        _ => None,
    }
}

/// Root callers (no parent) must not self-execute fallback -- Comet isolation
/// rules forbid the root/main session from executing tasks directly.
pub fn is_root_caller(context: &AgentExecutionContext) -> bool {
    context.parent_id.is_none()
}

/// A prepared structural fallback: a ghost leaf context for inline
/// self-execution, plus the ghost's agent id (useful for progress tracking).
///
/// The ghost is NOT registered in coordinator scopes. Callers must run a leaf
/// loop (spawn tools stripped) and must not call `finish_child` on it.
pub struct FallbackPrepared {
    /// Ghost leaf context with `parent_id = None` so synthesis treats it as a
    /// root-like leaf and skips `collect_children_for_synthesis`.
    pub ghost: AgentExecutionContext,
    /// The ghost agent's id (also embedded in `ghost.agent_id`).
    pub agent_id: String,
}

/// Why a structural fallback could not be prepared.
#[derive(Debug)]
pub enum FallbackBlocked {
    /// Root callers (no parent) must not self-execute -- Comet isolation.
    RootCaller,
    /// Single-shot constraint: fallback already used for this key.
    AlreadyUsed,
}

impl std::fmt::Display for FallbackBlocked {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FallbackBlocked::RootCaller => write!(f, "root caller cannot self-execute"),
            FallbackBlocked::AlreadyUsed => write!(f, "fallback already used for this child"),
        }
    }
}

/// Prepare a structural fallback for a blocked subagent dispatch.
///
/// This is the shared interception-point-1 helper used by the `task`,
/// `delegate`, and `run_script` tools. It enforces the two guards (root-caller
/// rejection and single-shot constraint), marks the fallback as used, and
/// synthesizes a ghost leaf context (`parent_id = None`, `depth = caller.depth`)
/// for inline self-execution.
///
/// Callers determine eligibility first via
/// [`fallback_eligible_from_coordinator_error`]; only structural errors
/// (`DepthLimitReached` / `ConcurrencyClosed` / `TaskGroup`) reach this point.
pub async fn prepare_structural_fallback(
    coordinator: &AgentCoordinator,
    caller: &AgentExecutionContext,
    fallback_key: &str,
) -> Result<FallbackPrepared, FallbackBlocked> {
    if is_root_caller(caller) {
        return Err(FallbackBlocked::RootCaller);
    }
    if coordinator.fallback_already_used(fallback_key).await {
        return Err(FallbackBlocked::AlreadyUsed);
    }
    coordinator.mark_fallback_used(fallback_key).await;

    let agent_id = uuid::Uuid::new_v4().to_string();
    let ghost = AgentExecutionContext {
        agent_id: AgentId::new(agent_id.clone()),
        parent_id: None,
        session_id: caller.session_id.clone(),
        depth: caller.depth,
        cancellation: caller.cancellation.child_token(),
    };
    Ok(FallbackPrepared { ghost, agent_id })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::coordinator::{ChildResult, ChildTerminalStatus};
    use crate::agent::identity::{AgentExecutionContext, AgentId, SessionId};
    use tokio_util::sync::CancellationToken;

    fn make_child_result(code: Option<&str>, status: ChildTerminalStatus) -> ChildResult {
        ChildResult {
            child_id: AgentId::new("child-1"),
            status,
            summary: String::new(),
            error_code: code.map(String::from),
            partial_result: None,
        }
    }

    fn make_context(parent: Option<&str>) -> AgentExecutionContext {
        AgentExecutionContext {
            agent_id: AgentId::new("agent-1"),
            parent_id: parent.map(AgentId::new),
            session_id: SessionId::new("s1"),
            depth: 0,
            cancellation: CancellationToken::new(),
        }
    }

    #[test]
    fn coordinator_depth_limit_is_structural() {
        let e = CoordinatorError::DepthLimitReached { limit: 5 };
        assert_eq!(
            fallback_eligible_from_coordinator_error(&e),
            Some(FallbackKind::Structural)
        );
    }

    #[test]
    fn coordinator_concurrency_closed_is_structural() {
        let e = CoordinatorError::ConcurrencyClosed;
        assert_eq!(
            fallback_eligible_from_coordinator_error(&e),
            Some(FallbackKind::Structural)
        );
    }

    #[test]
    fn coordinator_task_group_is_structural() {
        let e = CoordinatorError::TaskGroup("group gone".to_string());
        assert_eq!(
            fallback_eligible_from_coordinator_error(&e),
            Some(FallbackKind::Structural)
        );
    }

    #[test]
    fn coordinator_not_visible_not_eligible() {
        let e = CoordinatorError::NotVisible;
        assert_eq!(fallback_eligible_from_coordinator_error(&e), None);
    }

    #[test]
    fn coordinator_parent_not_running_not_eligible() {
        let e = CoordinatorError::ParentNotRunning;
        assert_eq!(fallback_eligible_from_coordinator_error(&e), None);
    }

    #[test]
    fn child_model_unavailable_is_model_fallback() {
        let r = make_child_result(
            Some("subagent_model_unavailable"),
            ChildTerminalStatus::Failed,
        );
        assert_eq!(
            fallback_eligible_from_child_result(&r),
            Some(FallbackKind::ModelUnavailable)
        );
    }

    #[test]
    fn child_timeout_not_eligible() {
        let r = make_child_result(Some("subagent_timeout"), ChildTerminalStatus::Failed);
        assert_eq!(fallback_eligible_from_child_result(&r), None);
    }

    #[test]
    fn child_stuck_not_eligible() {
        let r = make_child_result(Some("subagent_stuck"), ChildTerminalStatus::Failed);
        assert_eq!(fallback_eligible_from_child_result(&r), None);
    }

    #[test]
    fn child_cancelled_not_eligible() {
        let r = make_child_result(Some("subagent_cancelled"), ChildTerminalStatus::Failed);
        assert_eq!(fallback_eligible_from_child_result(&r), None);
    }

    #[test]
    fn child_generic_error_not_eligible() {
        let r = make_child_result(Some("subagent_error"), ChildTerminalStatus::Failed);
        assert_eq!(fallback_eligible_from_child_result(&r), None);
    }

    #[test]
    fn child_completed_not_eligible() {
        let r = make_child_result(None, ChildTerminalStatus::Completed);
        assert_eq!(fallback_eligible_from_child_result(&r), None);
    }

    #[test]
    fn child_no_error_code_not_eligible() {
        let r = make_child_result(None, ChildTerminalStatus::Failed);
        assert_eq!(fallback_eligible_from_child_result(&r), None);
    }

    #[test]
    fn root_caller_detected() {
        let ctx = make_context(None);
        assert!(is_root_caller(&ctx));
    }

    #[test]
    fn non_root_caller_detected() {
        let ctx = make_context(Some("root"));
        assert!(!is_root_caller(&ctx));
    }

    #[tokio::test]
    async fn prepare_fallback_rejects_root_caller() {
        let coordinator = AgentCoordinator::new(4, 1);
        let root = AgentExecutionContext::root(SessionId::new("s"));
        let result = prepare_structural_fallback(&coordinator, &root, "pending:x").await;
        assert!(matches!(result, Err(FallbackBlocked::RootCaller)));
    }

    #[tokio::test]
    async fn prepare_fallback_rejects_already_used() {
        use crate::agent::SpawnChildRequest;
        let coordinator = AgentCoordinator::new(4, 1);
        let root = coordinator.ensure_root(SessionId::new("s")).await.unwrap();
        let child = coordinator
            .reserve_child(&root, SpawnChildRequest::new("c"))
            .await
            .unwrap()
            .context;
        coordinator.mark_fallback_used("pending:x").await;
        let result = prepare_structural_fallback(&coordinator, &child, "pending:x").await;
        assert!(matches!(result, Err(FallbackBlocked::AlreadyUsed)));
    }

    #[tokio::test]
    async fn prepare_fallback_builds_ghost_context() {
        use crate::agent::SpawnChildRequest;
        let coordinator = AgentCoordinator::new(4, 1);
        let root = coordinator.ensure_root(SessionId::new("s")).await.unwrap();
        let child = coordinator
            .reserve_child(&root, SpawnChildRequest::new("c"))
            .await
            .unwrap()
            .context;
        let prepared = prepare_structural_fallback(&coordinator, &child, "pending:x")
            .await
            .expect("non-root caller with unused key must prepare");
        assert!(prepared.ghost.parent_id.is_none(), "ghost must be a leaf");
        assert_eq!(
            prepared.ghost.depth, child.depth,
            "ghost inherits caller depth"
        );
        assert_eq!(prepared.ghost.session_id, child.session_id);
        assert!(!prepared.agent_id.is_empty());
        assert!(
            coordinator.fallback_already_used("pending:x").await,
            "marker must be set after prepare"
        );
    }
}
