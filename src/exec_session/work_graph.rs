//! Code-owned routing for the fixed compile → test → verify work graph.

use crate::agent::coordinator::CoordinatorError;
use crate::org_graph::{NodeType, VerifyFailureKind, WorkState};
use serde::{Deserialize, Serialize};

/// The next fixed work-graph step selected from anchored state.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum WorkGraphStep {
    Implement,
    CompileAnchor,
    TestAnchor,
    VerifyGate,
    Complete,
    Escalate,
}

/// Select the next step exclusively from structured external-anchor results.
pub fn next_step(state: &WorkState) -> Result<WorkGraphStep, CoordinatorError> {
    let compile = state.compile_result(NodeType::Verification)?;
    let Some(compile) = compile else {
        return Ok(WorkGraphStep::CompileAnchor);
    };
    if !compile.ok {
        return retry_or_escalate(state);
    }

    let test = state.test_result(NodeType::Verification)?;
    let Some(test) = test else {
        return Ok(WorkGraphStep::TestAnchor);
    };
    if !test.pass {
        return retry_or_escalate(state);
    }

    let verify = state.verify_result(NodeType::Verification)?;
    let Some(verify) = verify else {
        return Ok(WorkGraphStep::VerifyGate);
    };
    if verify.success {
        return Ok(WorkGraphStep::Complete);
    }
    if matches!(
        verify.fail_reason,
        Some(VerifyFailureKind::BoundaryViolation { .. })
    ) {
        return Ok(WorkGraphStep::Escalate);
    }

    retry_or_escalate(state)
}

fn retry_or_escalate(state: &WorkState) -> Result<WorkGraphStep, CoordinatorError> {
    let budget = state.budget(NodeType::GeneralPurpose)?;
    Ok(match budget {
        Some(budget) if budget.iter_used < budget.max_iter => WorkGraphStep::Implement,
        _ => WorkGraphStep::Escalate,
    })
}

#[cfg(test)]
mod tests {
    use crate::org_graph::{
        Budget, CompileResult, NodeType, TestResult, VerifyFailureKind, VerifyOutcome, WorkState,
    };

    use super::{next_step, WorkGraphStep};

    #[test]
    fn failed_compile_routes_to_implement_when_budget_remains() {
        let mut state = WorkState::default();
        state
            .set_budget(
                NodeType::GeneralPurpose,
                Budget {
                    max_iter: 2,
                    iter_used: 1,
                    token_used: 0,
                },
            )
            .expect("coordinator work node may record budget");
        state
            .set_compile_result(
                NodeType::Verification,
                CompileResult {
                    ok: false,
                    stderr: "compile failed".into(),
                },
            )
            .expect("verification anchor may record compile failure");

        assert_eq!(
            next_step(&state).expect("route result"),
            WorkGraphStep::Implement
        );
    }

    #[test]
    fn boundary_violation_escalates_without_retry() {
        let mut state = WorkState::default();
        state
            .set_compile_result(
                NodeType::Verification,
                CompileResult {
                    ok: true,
                    stderr: String::new(),
                },
            )
            .expect("record compile result");
        state
            .set_test_result(
                NodeType::Verification,
                TestResult {
                    pass: true,
                    failed_cases: Vec::new(),
                },
            )
            .expect("record test result");
        state
            .set_verify_result(
                NodeType::Verification,
                VerifyOutcome {
                    success: false,
                    fail_reason: Some(VerifyFailureKind::BoundaryViolation {
                        unexpected_files: vec!["unexpected.rs".into()],
                    }),
                },
            )
            .expect("record verification result");

        assert_eq!(
            next_step(&state).expect("route result"),
            WorkGraphStep::Escalate
        );
    }

    #[test]
    fn passing_compile_and_test_require_final_verification() {
        let mut state = WorkState::default();
        state
            .set_compile_result(
                NodeType::Verification,
                CompileResult {
                    ok: true,
                    stderr: String::new(),
                },
            )
            .expect("record compile result");
        state
            .set_test_result(
                NodeType::Verification,
                TestResult {
                    pass: true,
                    failed_cases: Vec::new(),
                },
            )
            .expect("record test result");

        assert_eq!(
            next_step(&state).expect("route result"),
            WorkGraphStep::VerifyGate
        );
    }
}
