//! Fallback eligibility for subagent dispatch failures.
//!
//! Two interception points share this logic:
//! - Interception 1 (pre-dispatch): `CoordinatorError` from `reserve_child_in_group`
//! - Interception 2 (runtime): `ChildResult` with `error_code = subagent_model_unavailable`

use crate::agent::coordinator::{ChildResult, ChildTerminalStatus, CoordinatorError};
use crate::agent::identity::AgentExecutionContext;

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
        let r = make_child_result(Some("subagent_model_unavailable"), ChildTerminalStatus::Failed);
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
}
