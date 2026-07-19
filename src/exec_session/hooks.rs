//! Session hooks: extension points the inner layer calls at well-defined
//! boundaries.
//!
//! The inner layer never probes for skill presence. The caller (outer
//! ExecutionSession or a skill adapter) supplies a [`SessionHooks`] impl; the
//! coordinator / verify-gate invokes it. Default impls are safe no-ops or the
//! spec-mandated `AutoRetry { max: 2 }` (§3.3), so callers only override what
//! they need.
//!
//! Task 2 scope: type definitions + trait + `NoHooks` default. The
//! `verify_fail` invocation path is wired in Task 5/6; `pre_node` /
//! `post_node` are reserved for the outer node state machine (§5).

/// Why the verify-gate failed. Passed to [`SessionHooks::verify_fail`] so the
/// hook (and, through it, the agent) can decide how to react.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifyFailure {
    /// A verification command exited non-zero.
    CommandFailed {
        command: String,
        exit_code: Option<i32>,
        stderr: String,
    },
    /// Files changed outside the agent-declared `expected_changed_files` set.
    BoundaryViolation { unexpected_files: Vec<String> },
}

/// Context handed to [`SessionHooks::verify_fail`] on a gate failure.
///
/// `attempt` is 1-based: the first `verify_and_complete` call is attempt 1,
/// the first retry is attempt 2, etc. The default hook uses this to compute
/// the remaining retry budget.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyFailContext {
    pub session_id: String,
    pub turn_id: String,
    pub attempt: usize,
    pub failure: VerifyFailure,
}

/// Context handed to [`SessionHooks::rollback_triggered`] when
/// [`super::SessionCoordinator::rollback_to`] restores the workspace to a
/// prior turn (spec §3.4).
///
/// `from_turn` is the cursor before the rollback (`None` if no turn was
/// active); `to_turn` is the target turn the workspace is being restored to.
/// The hook is informational - the rollback proceeds regardless - so the
/// outer layer can observe / log / audit rollback events.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RollbackContext {
    pub session_id: String,
    pub from_turn: Option<String>,
    pub to_turn: String,
}

/// What the runtime should do after a verify-gate failure (spec §3.3).
///
/// Core principle: gate failure is a signal, not a punishment. The runtime
/// never auto-rolls-back on failure - rollback is the agent's explicit tool
/// (`rollback_to`, Task 4). This enum only controls whether the agent gets
/// another attempt, is escalated, or (rarely) the failure is accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifyFailAction {
    /// Let the agent try again: it sees the failure reason and can
    /// self-correct. `remaining` is how many more attempts the agent has
    /// before escalation kicks in.
    AutoRetry { remaining: usize },
    /// Escalate to the outer orchestration layer (comet/plan) or human.
    /// Session status transitions to `Failed`; workspace changes are
    /// preserved (not rolled back).
    Escalate,
    /// Abort the session immediately. Rare; for hooks that decide the session
    /// is unrecoverable (e.g. guardian blocked a critical command).
    Abort,
    /// Accept the failure and mark the session `Completed` anyway. For hooks
    /// that decide the failure is acceptable (e.g. a known flaky test, a
    /// pre-existing failure outside the agent's scope). The `verify_log`
    /// records the failed attempt; `final_status` becomes `Completed` by hook
    /// decision, so the outcome is auditable. Use sparingly - it overrides the
    /// gate's anti-fabrication guarantee by hook authority.
    WarnAndContinue,
}

/// Extension points injected by the caller. The inner layer calls these at
/// well-defined points; it never branches on session source or skill name.
///
/// All methods have defaults, so a caller can implement only the hooks it
/// needs. [`NoHooks`] uses all defaults.
pub trait SessionHooks: Send + Sync {
    /// Called when `verify_and_complete` (Task 5) fails. Default: `AutoRetry`
    /// with a budget of 2 additional attempts (spec §3.3: "默认
    /// AutoRetry{max:2}"). After the budget is exhausted, returns `Escalate`.
    ///
    /// Semantics: `max: 2` means the agent may call `verify_and_complete` 2
    /// more times after the first attempt (3 total). The hook is stateless -
    /// it derives the remaining budget from `ctx.attempt`.
    fn verify_fail(&self, ctx: &VerifyFailContext) -> VerifyFailAction {
        const MAX_RETRIES: usize = 2;
        // attempt 1 (first try) -> remaining 2; attempt 2 -> remaining 1;
        // attempt 3 -> Escalate (budget exhausted).
        let remaining = (MAX_RETRIES + 1).saturating_sub(ctx.attempt);
        if remaining > 0 {
            VerifyFailAction::AutoRetry { remaining }
        } else {
            VerifyFailAction::Escalate
        }
    }

    /// Reserved for the outer ExecutionSession node state machine (§5). The
    /// inner layer does not call this; default is a no-op.
    fn pre_node(&self, _node_id: &str) {}

    /// Reserved for the outer ExecutionSession node state machine (§5). The
    /// inner layer does not call this; default is a no-op.
    fn post_node(&self, _node_id: &str) {}

    /// Called when [`super::SessionCoordinator::rollback_to`] (Task 4) is
    /// about to restore the workspace to a prior turn (spec §3.4). Default:
    /// no-op (observe only). The rollback proceeds regardless - this hook is
    /// informational, letting the outer layer log / audit / surface rollback
    /// events. It does NOT gate the rollback.
    fn rollback_triggered(&self, _ctx: &RollbackContext) {}
}

/// No-op hooks: uses the default `AutoRetry { max: 2 }` `verify_fail` and
/// empty `pre_node` / `post_node`. Callers that don't need custom hooking
/// pass this (or `Arc<NoHooks>`).
#[derive(Debug, Clone)]
pub struct NoHooks;

impl SessionHooks for NoHooks {}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(attempt: usize, failure: VerifyFailure) -> VerifyFailContext {
        VerifyFailContext {
            session_id: "es-test".into(),
            turn_id: "turn-0".into(),
            attempt,
            failure,
        }
    }

    fn cmd_failure() -> VerifyFailure {
        VerifyFailure::CommandFailed {
            command: "cargo test".into(),
            exit_code: Some(1),
            stderr: "1 test failed".into(),
        }
    }

    fn boundary_failure() -> VerifyFailure {
        VerifyFailure::BoundaryViolation {
            unexpected_files: vec!["src/other.rs".into()],
        }
    }

    #[test]
    fn default_verify_fail_attempt_1_yields_two_remaining() {
        let hooks = NoHooks;
        let action = hooks.verify_fail(&ctx(1, cmd_failure()));
        assert_eq!(action, VerifyFailAction::AutoRetry { remaining: 2 });
    }

    #[test]
    fn default_verify_fail_attempt_2_yields_one_remaining() {
        let hooks = NoHooks;
        let action = hooks.verify_fail(&ctx(2, cmd_failure()));
        assert_eq!(action, VerifyFailAction::AutoRetry { remaining: 1 });
    }

    #[test]
    fn default_verify_fail_attempt_3_escalates() {
        let hooks = NoHooks;
        let action = hooks.verify_fail(&ctx(3, cmd_failure()));
        assert_eq!(action, VerifyFailAction::Escalate);
    }

    #[test]
    fn default_verify_fail_beyond_budget_escalates() {
        let hooks = NoHooks;
        let action = hooks.verify_fail(&ctx(10, cmd_failure()));
        assert_eq!(action, VerifyFailAction::Escalate);
    }

    #[test]
    fn default_verify_fail_boundary_violation_same_budget() {
        // Boundary violations use the same retry budget as command failures.
        let hooks = NoHooks;
        assert_eq!(
            hooks.verify_fail(&ctx(1, boundary_failure())),
            VerifyFailAction::AutoRetry { remaining: 2 }
        );
        assert_eq!(
            hooks.verify_fail(&ctx(3, boundary_failure())),
            VerifyFailAction::Escalate
        );
    }

    #[test]
    fn custom_hook_can_override_to_abort() {
        struct AlwaysAbort;
        impl SessionHooks for AlwaysAbort {
            fn verify_fail(&self, _ctx: &VerifyFailContext) -> VerifyFailAction {
                VerifyFailAction::Abort
            }
        }
        let hooks = AlwaysAbort;
        assert_eq!(
            hooks.verify_fail(&ctx(1, cmd_failure())),
            VerifyFailAction::Abort
        );
    }

    #[test]
    fn custom_hook_can_override_budget() {
        struct RetryThree;
        impl SessionHooks for RetryThree {
            fn verify_fail(&self, ctx: &VerifyFailContext) -> VerifyFailAction {
                const MAX: usize = 3;
                let remaining = (MAX + 1).saturating_sub(ctx.attempt);
                if remaining > 0 {
                    VerifyFailAction::AutoRetry { remaining }
                } else {
                    VerifyFailAction::Escalate
                }
            }
        }
        let hooks = RetryThree;
        assert_eq!(
            hooks.verify_fail(&ctx(1, cmd_failure())),
            VerifyFailAction::AutoRetry { remaining: 3 }
        );
        assert_eq!(
            hooks.verify_fail(&ctx(4, cmd_failure())),
            VerifyFailAction::Escalate
        );
    }

    #[test]
    fn pre_node_and_post_node_default_noop() {
        let hooks = NoHooks;
        // Just ensure they don't panic.
        hooks.pre_node("node-0");
        hooks.post_node("node-0");
    }

    #[test]
    fn rollback_triggered_default_noop() {
        let hooks = NoHooks;
        hooks.rollback_triggered(&RollbackContext {
            session_id: "es-test".into(),
            from_turn: Some("turn-1".into()),
            to_turn: "turn-0".into(),
        });
        // Default is a no-op; nothing to assert beyond not panicking.
    }

    #[test]
    fn custom_hook_can_observe_rollback_context() {
        use std::sync::Mutex;

        #[derive(Default)]
        struct RecordingHook {
            seen: Mutex<Vec<RollbackContext>>,
        }
        impl SessionHooks for RecordingHook {
            fn rollback_triggered(&self, ctx: &RollbackContext) {
                self.seen.lock().unwrap().push(ctx.clone());
            }
        }

        let hooks = RecordingHook::default();
        let ctx = RollbackContext {
            session_id: "es-obs".into(),
            from_turn: Some("turn-2".into()),
            to_turn: "turn-0".into(),
        };
        hooks.rollback_triggered(&ctx);
        let seen = hooks.seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0], ctx);
    }
}
