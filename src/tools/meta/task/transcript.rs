use crate::agent::progress::{ErrorInfo, ErrorType, SubagentStatus};
use crate::teams::subagent_loop::{build_failure_diagnostics, SubagentError};
use crate::transcript::{
    SubagentEventRecord, SubagentTranscript, SubagentTranscriptStore, TranscriptStatus,
};

/// Helper to build a SubagentTranscript from subagent execution metadata.
#[allow(clippy::too_many_arguments)]
pub(super) fn build_transcript(
    id: String,
    session_id: &str,
    description: &str,
    status: TranscriptStatus,
    system_prompt: Option<String>,
    user_prompt: String,
    started_at: i64,
    total_tokens: u64,
    actual_rounds: u32,
    token_budget_k: Option<u64>,
    error_message: Option<String>,
    summary: Option<String>,
    events: Vec<SubagentEventRecord>,
    failure_diagnostics: Option<ErrorInfo>,
    project_path: Option<String>,
) -> SubagentTranscript {
    SubagentTranscript {
        id,
        session_id: session_id.to_string(),
        parent_id: None,
        label: format!("task: {}", description),
        status,
        system_prompt,
        user_prompt,
        started_at,
        finished_at: Some(chrono::Utc::now().timestamp_millis()),
        total_tokens,
        max_rounds: Some(100),
        actual_rounds,
        token_budget_k,
        error_message,
        summary,
        failure_diagnostics,
        project_path,
        events,
    }
}

/// Save a minimal transcript for subagent paths that don't have full
/// progress-store telemetry (fallback, run_script, RLM pipeline,
/// attempt_model_fallback). Builds `failure_diagnostics` from the
/// `SubagentError` result so root-cause classification is never NULL.
///
/// On success, saves a `Completed` transcript with the summary. On failure,
/// saves a `Failed` transcript with `error_message` and structured
/// `failure_diagnostics` (root_cause classified from the error message,
/// empty tool sequence / round context since no observer data is available).
#[allow(clippy::too_many_arguments)]
pub(crate) fn save_minimal_transcript(
    store: &SubagentTranscriptStore,
    id: &str,
    session_id: &str,
    description: &str,
    system_prompt: Option<String>,
    user_prompt: String,
    started_at: i64,
    result: &Result<String, SubagentError>,
    context_char_limit: usize,
    retention_days: Option<u32>,
) {
    let (status, error_message, summary, failure_diagnostics) = match result {
        Ok(text) => (
            TranscriptStatus::Completed,
            None,
            Some(text.chars().take(500).collect()),
            None,
        ),
        Err(e) => {
            let msg = e.full_message();
            let status = if matches!(e.error_type, ErrorType::Cancelled) {
                SubagentStatus::Cancelled
            } else {
                SubagentStatus::Failed
            };
            let diag =
                build_failure_diagnostics(&msg, status.clone(), &[], None, context_char_limit);
            let info = ErrorInfo {
                error_type: e.error_type.clone(),
                message: msg.clone(),
                last_tool: None,
                last_params: None,
                round: 0,
                retryable: true,
                root_cause: diag.root_cause,
                failed_tool_sequence: diag.failed_tool_sequence,
                failed_round_context: diag.failed_round_context,
                retry_history: diag.retry_history,
            };
            (TranscriptStatus::Failed, Some(msg), None, Some(info))
        }
    };
    let project_path = std::env::current_dir()
        .ok()
        .map(|p| p.to_string_lossy().to_string());
    let transcript = build_transcript(
        id.to_string(),
        session_id,
        description,
        status,
        system_prompt,
        user_prompt,
        started_at,
        0,
        0,
        None,
        error_message,
        summary,
        Vec::new(),
        failure_diagnostics,
        project_path,
    );
    let _ = store.save(&transcript, retention_days);
}

/// Generate a UUID string for a new transcript id.
pub(crate) fn new_transcript_id() -> String {
    uuid::Uuid::new_v4().to_string()
}
