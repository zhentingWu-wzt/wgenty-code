use crate::agent::progress::ErrorInfo;
use crate::transcript::{SubagentEventRecord, SubagentTranscript, TranscriptStatus};

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
