//! Code-owned routing for the fixed compile → test → verify work graph.

use crate::agent::coordinator::CoordinatorError;
use crate::org_graph::{HumanReview, NodeType, SpecialistReportKind, VerifyFailureKind, WorkState};
use serde::{Deserialize, Serialize};

/// The next fixed work-graph step selected from anchored state.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum WorkGraphStep {
    /// Run the predeclared root-cause specialist before modifying code again.
    RootCause,
    Implement,
    CompileAnchor,
    TestAnchor,
    VerifyGate,
    /// Wait for the external HumanReview veto gate selected in this graph.
    AwaitHumanReview,
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
        if state.selected_work_graph().is_some_and(|plan| {
            plan.nodes
                .iter()
                .any(|node| node.role == NodeType::HumanReview)
        }) {
            return match state.human_review(NodeType::HumanReview)? {
                None => Ok(WorkGraphStep::AwaitHumanReview),
                Some(HumanReview::Approve) => Ok(WorkGraphStep::Complete),
                Some(HumanReview::Reject) => retry_or_escalate(state),
            };
        }
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
    if !matches!(budget, Some(budget) if budget.iter_used < budget.max_iter) {
        return Ok(WorkGraphStep::Escalate);
    }

    let reports = state.specialist_reports(NodeType::GeneralPurpose)?;
    Ok(
        if reports.iter().any(|report| {
            report.producer == NodeType::RootCause && report.kind == SpecialistReportKind::RootCause
        }) {
            WorkGraphStep::Implement
        } else {
            WorkGraphStep::RootCause
        },
    )
}

#[cfg(test)]
mod tests {
    use crate::org_graph::{
        Budget, CompileResult, NodeType, TestResult, VerifyFailureKind, VerifyOutcome, WorkState,
    };

    use super::{next_step, WorkGraphStep};

    #[test]
    fn failed_compile_routes_to_root_cause_when_budget_remains() {
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
            WorkGraphStep::RootCause
        );
    }

    #[test]
    fn retryable_failure_routes_to_implement_only_after_root_cause_handoff() {
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
        state
            .set_specialist_report(
                NodeType::RootCause,
                crate::org_graph::SpecialistReport {
                    producer: NodeType::RootCause,
                    kind: crate::org_graph::SpecialistReportKind::RootCause,
                    summary: "The guard runs after the fallible branch.".into(),
                    evidence: vec![crate::org_graph::SpecialistEvidence {
                        path: "src/guard.rs".into(),
                        detail: "The branch returns before validation.".into(),
                    }],
                    suspected_files: vec!["src/guard.rs".into()],
                    recommended_actions: vec!["Validate before branching.".into()],
                },
            )
            .expect("root cause handoff");

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
