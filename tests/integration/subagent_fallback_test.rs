//! Integration tests for subagent dispatch fallback.
//!
//! These exercise the public API surface of the fallback feature end-to-end
//! at the library boundary: error classification, code mapping, fallback
//! eligibility, fallback model selection, and health-panel classification.
//! Full re-dispatch (which needs a live model endpoint) is covered by the
//! unit tests on SubagentSynthesis and the interception-point code paths.

use wgenty_code::agent::coordinator::{ChildResult, ChildTerminalStatus, CoordinatorError};
use wgenty_code::agent::fallback::{
    fallback_eligible_from_child_result, fallback_eligible_from_coordinator_error, FallbackKind,
};
use wgenty_code::agent::identity::AgentId;
use wgenty_code::agent::progress::ErrorType;
use wgenty_code::config::Settings;
use wgenty_code::teams::subagent_health::FailureMode;
use wgenty_code::teams::subagent_loop::{classify_stream_error, SubagentError};

#[test]
fn integration_model_unavailable_classification() {
    // Simulate a deepseek-reasoner 503 error string (as produced by llm_api.rs).
    let msg = "API error (503): service unavailable";
    assert_eq!(classify_stream_error(msg), ErrorType::ModelUnavailable);

    let err = SubagentError {
        message: msg.to_string(),
        error_type: ErrorType::ModelUnavailable,
        partial_result: None,
    };
    assert_eq!(err.code(), "subagent_model_unavailable");
}

#[test]
fn integration_connection_error_classified_as_model_unavailable() {
    assert_eq!(
        classify_stream_error("connection refused by upstream"),
        ErrorType::ModelUnavailable
    );
}

#[test]
fn integration_non_model_stream_error_stays_unknown() {
    assert_eq!(
        classify_stream_error("some unexpected stream error"),
        ErrorType::Unknown
    );
}

#[test]
fn integration_health_panel_classifies_model_unavailable() {
    assert_eq!(
        FailureMode::classify("subagent_model_unavailable"),
        FailureMode::ModelUnavailable
    );
    assert_eq!(FailureMode::ModelUnavailable.severity(), "Critical");
    assert_eq!(FailureMode::ModelUnavailable.label(), "Model Unavailable");
}

#[test]
fn integration_fallback_model_selection_picks_first_different() {
    let mut settings = Settings::default();
    settings.models.main.name = "deepseek-reasoner".to_string();
    settings.agent.subagent.fallback_models = vec![
        "deepseek-reasoner".to_string(),
        "claude-sonnet-4".to_string(),
        "gpt-4o".to_string(),
    ];
    assert_eq!(
        settings.select_fallback_model("deepseek-reasoner"),
        Some("claude-sonnet-4")
    );
}

#[test]
fn integration_fallback_model_settings_only_swaps_name() {
    let mut settings = Settings::default();
    settings.models.main.name = "deepseek-reasoner".to_string();
    settings.models.main.base_url = Some("https://api.deepseek.com".to_string());
    settings.models.main.api_key = Some("sk-deepseek".to_string());

    let fb = settings.fallback_model_settings("claude-sonnet-4");
    assert_eq!(fb.models.main.name, "claude-sonnet-4");
    // Endpoint reused (not swapped).
    assert_eq!(
        fb.models.main.base_url,
        Some("https://api.deepseek.com".to_string())
    );
    assert_eq!(fb.models.main.api_key, Some("sk-deepseek".to_string()));
}

#[test]
fn integration_no_fallback_config_degrades() {
    let settings = Settings::default();
    assert!(settings.agent.subagent.fallback_models.is_empty());
    assert_eq!(settings.select_fallback_model("any-model"), None);
}

#[test]
fn integration_structural_error_eligibility() {
    assert_eq!(
        fallback_eligible_from_coordinator_error(&CoordinatorError::DepthLimitReached { limit: 1 }),
        Some(FallbackKind::Structural)
    );
    assert_eq!(
        fallback_eligible_from_coordinator_error(&CoordinatorError::ConcurrencyClosed),
        Some(FallbackKind::Structural)
    );
    assert_eq!(
        fallback_eligible_from_coordinator_error(&CoordinatorError::TaskGroup(
            "group gone".to_string()
        )),
        Some(FallbackKind::Structural)
    );
}

#[test]
fn integration_non_eligible_coordinator_errors() {
    assert_eq!(
        fallback_eligible_from_coordinator_error(&CoordinatorError::NotVisible),
        None
    );
    assert_eq!(
        fallback_eligible_from_coordinator_error(&CoordinatorError::ParentNotRunning),
        None
    );
}

#[test]
fn integration_child_result_model_unavailable_eligible() {
    let r = ChildResult {
        child_id: AgentId::new("c1"),
        status: ChildTerminalStatus::Failed,
        summary: String::new(),
        error_code: Some("subagent_model_unavailable".to_string()),
        partial_result: None,
    };
    assert_eq!(
        fallback_eligible_from_child_result(&r),
        Some(FallbackKind::ModelUnavailable)
    );
}

#[test]
fn integration_child_result_other_codes_not_eligible() {
    for code in [
        "subagent_timeout",
        "subagent_stuck",
        "subagent_cancelled",
        "subagent_error",
    ] {
        let r = ChildResult {
            child_id: AgentId::new("c1"),
            status: ChildTerminalStatus::Failed,
            summary: String::new(),
            error_code: Some(code.to_string()),
            partial_result: None,
        };
        assert_eq!(
            fallback_eligible_from_child_result(&r),
            None,
            "code {code} should not be fallback-eligible"
        );
    }
}

#[test]
fn integration_completed_child_not_eligible() {
    let r = ChildResult {
        child_id: AgentId::new("c1"),
        status: ChildTerminalStatus::Completed,
        summary: "done".to_string(),
        error_code: None,
        partial_result: None,
    };
    assert_eq!(fallback_eligible_from_child_result(&r), None);
}

#[tokio::test]
async fn integration_fallback_used_single_shot_constraint() {
    use wgenty_code::agent::AgentCoordinator;
    let coord = AgentCoordinator::new(4, 5);
    let key = "child-1";
    assert!(!coord.fallback_already_used(key).await);
    coord.mark_fallback_used(key).await;
    assert!(coord.fallback_already_used(key).await);
    // A second mark is idempotent; a different key is independent.
    coord.mark_fallback_used(key).await;
    assert!(!coord.fallback_already_used("child-2").await);
}
