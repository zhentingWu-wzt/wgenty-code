//! Subagent Loop — isolated agent loop for subagent execution.
//!
//! Control flow is [`crate::agent::runtime::run_agent_loop`]. This module
//! provides subagent-specific ports (guarding tools, progress observer,
//! non-root synthesis barrier) and exposes [`run_subagent_loop_with_permissions`]
//! for `task` / RLM / run_script callers.

use crate::agent::progress::{
    ErrorInfo, ErrorType, ProgressCallback, SubagentEvent, SubagentEventType, SubagentMetadata,
    SubagentProgress, SubagentStatus,
};
use crate::agent::runtime::{
    run_agent_loop, ApiLlmPort, EventSink, HistoryStore, InboxPort, LoopHooks, LoopTurnState,
    MutexHistoryStore, RoundObserver, RunLoopArgs, RuntimeConfig, RuntimeError, RuntimeEvent,
    StreamStyle, SynthesisPort,
};
use crate::agent::{
    AgentCoordinator, AgentExecutionContext, AgentId, ChildResult, ChildTerminalStatus,
    CoordinatorError, JoinPolicy,
};
use crate::api::{ApiClient, ChatMessage};
use crate::config::resolve_context_window;
use crate::teams::approval_registry;
use crate::teams::failure_diagnostics::{
    redact_params, truncate_char_safe, FailedRoundContext, FailureRootCause, RetryAttempt,
    RetryOutcome, ToolCallStep,
};
use crate::teams::guarding_tool_port::{
    format_permission_summary, GuardingToolPort, SubagentPermissionContext,
};
use crate::teams::mailbox::{Mailbox, TeamMessage};
use crate::teams::subagent_health::{FailureMode, FailureSignals};
use crate::teams::trace_sink::{compose_progress_callback, TraceSink};
use crate::tools::ToolRegistry;
use crate::utils::stuck_detector::StuckDetector;
use async_trait::async_trait;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use uuid::Uuid;

/// Maximum length of a tool parameter summary string.
const MAX_PARAMS_SUMMARY_LEN: usize = 80;

/// Extract a human-readable summary of the most meaningful tool parameters.
fn extract_params_summary(tool_name: &str, args: &serde_json::Value) -> String {
    let obj = match args.as_object() {
        Some(o) => o,
        None => return String::new(),
    };

    let keys: Vec<&str> = match tool_name {
        "file_read" | "read_file" | "file_write" | "write_file" => {
            vec!["file_path"]
        }
        "grep" | "search" => {
            if obj.contains_key("path") {
                vec!["pattern", "path"]
            } else {
                vec!["pattern"]
            }
        }
        "glob" | "file_glob" => {
            vec!["pattern"]
        }
        "execute_command" | "exec_command" | "shell" => {
            vec!["command"]
        }
        "web_fetch" | "web_search" => {
            vec!["url", "query"]
        }
        "task" | "delegate" => {
            vec!["description"]
        }
        "edit" | "file_edit" | "write" => {
            vec!["file_path"]
        }
        _ => obj.keys().map(|s| s.as_str()).take(2).collect(),
    };

    let parts: Vec<String> = keys
        .iter()
        .filter_map(|&k| {
            obj.get(k).map(|v| {
                let s = match v {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                if s.len() > MAX_PARAMS_SUMMARY_LEN {
                    let end = s.floor_char_boundary(MAX_PARAMS_SUMMARY_LEN);
                    format!("{}…", &s[..end])
                } else {
                    s
                }
            })
        })
        .collect();

    parts.join(", ")
}

/// Structured error returned by [`run_subagent_loop_with_permissions`] when a subagent fails.
#[derive(Debug, Clone)]
pub struct SubagentError {
    pub message: String,
    pub error_type: ErrorType,
    pub partial_result: Option<String>,
}

impl SubagentError {
    pub fn full_message(&self) -> String {
        match &self.partial_result {
            Some(partial) if !partial.trim().is_empty() => {
                format!(
                    "{}\n\n--- Partial results (before failure) ---\n{}",
                    self.message, partial
                )
            }
            _ => self.message.clone(),
        }
    }

    pub fn code(&self) -> &'static str {
        match &self.error_type {
            ErrorType::BudgetExceeded { .. } => "budget_exceeded",
            ErrorType::Timeout => "subagent_timeout",
            ErrorType::Stuck { .. } => "subagent_stuck",
            ErrorType::ToolError { .. } => "subagent_tool_error",
            ErrorType::ParseError { .. } => "subagent_parse_error",
            ErrorType::Cancelled => "subagent_cancelled",
            ErrorType::ModelUnavailable => "subagent_model_unavailable",
            ErrorType::Unknown => "subagent_error",
        }
    }
}

/// Classify a free-form `RuntimeError::Stream` message into an `ErrorType`.
///
/// Model-unavailable signatures (API HTTP errors, connection failures) map to
/// [`ErrorType::ModelUnavailable`] so the fallback layer can detect them.
/// "stuck"/"Stuck" stays [`ErrorType::Stuck`]. Everything else stays
/// [`ErrorType::Unknown`].
pub fn classify_stream_error(msg: &str) -> ErrorType {
    if msg.contains("stuck") || msg.contains("Stuck") {
        return ErrorType::Stuck {
            reason: msg.to_string(),
        };
    }
    if is_model_unavailable_message(msg) {
        return ErrorType::ModelUnavailable;
    }
    ErrorType::Unknown
}

/// Heuristic: does this stream failure message indicate the model endpoint was
/// unavailable? Matches "API error", "api error", "connection", "HTTP <status>",
/// or a `(NNN)` status-code parenthetical.
fn is_model_unavailable_message(msg: &str) -> bool {
    use std::sync::OnceLock;
    static HTTP_STATUS: OnceLock<regex::Regex> = OnceLock::new();
    static STATUS_PAREN: OnceLock<regex::Regex> = OnceLock::new();
    let http_status = HTTP_STATUS.get_or_init(|| {
        // word-boundary "HTTP" followed by optional whitespace and 3 digits
        regex::Regex::new(r"(?i)\bHTTP\b\s*\d{3}").expect("valid regex")
    });
    let status_paren =
        STATUS_PAREN.get_or_init(|| regex::Regex::new(r"\(\d{3}\)").expect("valid regex"));
    let lower = msg.to_lowercase();
    lower.contains("api error")
        || lower.contains("connection")
        || http_status.is_match(msg)
        || status_paren.is_match(msg)
}

impl std::fmt::Display for SubagentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.full_message())
    }
}

impl From<String> for SubagentError {
    fn from(msg: String) -> Self {
        SubagentError {
            message: msg,
            error_type: ErrorType::Unknown,
            partial_result: None,
        }
    }
}

/// Structured failure diagnostics assembled at the capture site.
struct FailureDiagnostics {
    root_cause: FailureRootCause,
    failed_tool_sequence: Vec<ToolCallStep>,
    failed_round_context: Option<FailedRoundContext>,
    retry_history: Vec<RetryAttempt>,
}

/// Derive structured [`FailureSignals`] from capture-site data. Guardian
/// rejections come from the most recent `permission_denied` action-log event
/// (structured signal); sandbox/panic are best-effort matches on the error
/// message (the observer has no richer signal for those); user-cancellation
/// comes from the terminal status.
fn extract_failure_signals(
    error_msg: &str,
    status: SubagentStatus,
    action_log: &[SubagentEvent],
) -> FailureSignals {
    let mut signals = FailureSignals {
        user_cancelled: matches!(
            status,
            SubagentStatus::Cancelled | SubagentStatus::Cancelling
        ),
        ..Default::default()
    };
    for event in action_log.iter().rev() {
        if let SubagentEventType::Permission { kind, detail } = &event.event_type {
            if kind.contains("denied") || kind.contains("rejected") {
                signals.guardian_rejected_reason = Some(detail.clone());
                break;
            }
        }
    }
    let lower = error_msg.to_lowercase();
    signals.sandbox_failed = lower.contains("sandbox");
    signals.tool_panic = lower.contains("panic");
    signals
}

/// Build the failed-round tool-call sequence: the `Action` events after the
/// last `Thought` (the failing round), each paired with its `ToolResult` to
/// derive `elapsed_ms`. A trailing `Action` with no result emits with 0ms.
/// Parameter summaries are wrapped as JSON string values and run through
/// [`redact_params`] (no-op on string values, but keeps the contract for
/// future structured params).
fn build_failed_tool_sequence(action_log: &[SubagentEvent]) -> Vec<ToolCallStep> {
    let start = action_log
        .iter()
        .rposition(|e| matches!(e.event_type, SubagentEventType::Thought { .. }))
        .map(|i| i + 1)
        .unwrap_or(0);
    let mut steps = Vec::new();
    let mut pending: Option<(String, String, u64)> = None;
    for event in &action_log[start..] {
        match &event.event_type {
            SubagentEventType::Action {
                tool_name,
                params_summary,
            } => {
                if let Some((name, summary, _)) = pending.take() {
                    steps.push(ToolCallStep {
                        tool_name: name,
                        params_summary: redact_params(serde_json::Value::String(summary)),
                        elapsed_ms: 0,
                    });
                }
                pending = Some((tool_name.clone(), params_summary.clone(), event.elapsed_ms));
            }
            SubagentEventType::ToolResult { .. } => {
                if let Some((name, summary, start_elapsed)) = pending.take() {
                    steps.push(ToolCallStep {
                        tool_name: name,
                        params_summary: redact_params(serde_json::Value::String(summary)),
                        elapsed_ms: event.elapsed_ms.saturating_sub(start_elapsed),
                    });
                }
            }
            _ => {}
        }
    }
    if let Some((name, summary, _)) = pending {
        steps.push(ToolCallStep {
            tool_name: name,
            params_summary: redact_params(serde_json::Value::String(summary)),
            elapsed_ms: 0,
        });
    }
    steps
}

/// Build the failed-round context: the last `Thought` text (falling back to
/// `text_snapshot`) as `assistant_text`, and the last `ToolResult` summary as
/// `final_tool_output`. Both are char-boundary truncated to `context_char_limit`.
/// Returns `None` when neither assistant text nor a tool output is available.
fn build_failed_round_context(
    action_log: &[SubagentEvent],
    text_snapshot: Option<&str>,
    context_char_limit: usize,
) -> Option<FailedRoundContext> {
    let assistant_text = action_log
        .iter()
        .rev()
        .find_map(|e| match &e.event_type {
            SubagentEventType::Thought { text } => Some(text.as_str()),
            _ => None,
        })
        .or(text_snapshot);
    let final_tool_output = action_log.iter().rev().find_map(|e| match &e.event_type {
        SubagentEventType::ToolResult { summary, .. } => Some(summary.as_str()),
        _ => None,
    });
    let (text, output) = match (assistant_text, final_tool_output) {
        (Some(t), Some(o)) => (t, o),
        (Some(t), None) => (t, ""),
        (None, Some(o)) => ("", o),
        (None, None) => return None,
    };
    Some(FailedRoundContext {
        assistant_text: truncate_char_safe(text, context_char_limit),
        final_tool_output: truncate_char_safe(output, context_char_limit),
    })
}

/// A retry-attempt signal captured from the execution path. The subagent loop
/// has no in-loop subagent-level retry today (the only re-attempt is the
/// parent-level `attempt_model_fallback` model re-dispatch, which spawns a
/// separate child run rather than retrying within one `ErrorInfo`). When a
/// subagent-level retry path is added, emit one `RetrySignal` per attempt and
/// [`build_retry_history`] will record it.
#[derive(Debug, Clone)]
pub struct RetrySignal {
    pub error: String,
    pub root_cause: FailureRootCause,
    pub strategy: String,
    pub outcome: RetryOutcome,
}

/// Build the retry-history from captured retry signals. Empty when no retries
/// occurred (the common single-attempt case); one [`RetryAttempt`] per retry
/// signal otherwise.
fn build_retry_history(signals: &[RetrySignal]) -> Vec<RetryAttempt> {
    signals
        .iter()
        .map(|s| RetryAttempt {
            error: s.error.clone(),
            root_cause: s.root_cause.clone(),
            strategy: s.strategy.clone(),
            outcome: s.outcome,
        })
        .collect()
}

/// Assemble the full failure-diagnostics triple at the capture site. Pure
/// (no locks) so it is unit-testable; the observer passes its cloned
/// `action_log` and `text_snapshot`.
fn build_failure_diagnostics(
    error_msg: &str,
    status: SubagentStatus,
    action_log: &[SubagentEvent],
    text_snapshot: Option<&str>,
    context_char_limit: usize,
) -> FailureDiagnostics {
    let signals = extract_failure_signals(error_msg, status, action_log);
    let mode = FailureMode::classify_with_signals(error_msg, &signals);
    let root_cause = mode.to_root_cause(signals.guardian_rejected_reason.as_deref());
    let failed_tool_sequence = build_failed_tool_sequence(action_log);
    let failed_round_context =
        build_failed_round_context(action_log, text_snapshot, context_char_limit);
    FailureDiagnostics {
        root_cause,
        failed_tool_sequence,
        failed_round_context,
        // No in-loop subagent retry today -> empty history. When a retry path
        // is added, pass captured RetrySignals here instead of `&[]`.
        retry_history: build_retry_history(&[]),
    }
}

fn format_child_result_batch(results: &[ChildResult]) -> String {
    let body = serde_json::to_string(results).unwrap_or_else(|error| {
        format!(
            r#"{{"error":"serialize_child_results","message":{}}}"#,
            serde_json::json!(error.to_string())
        )
    });
    format!("<child-results>\n{body}\n</child-results>")
}

// ── Ports ───────────────────────────────────────────────────────────────────

struct NullEventSink;

impl EventSink for NullEventSink {
    fn emit(&self, event: RuntimeEvent) {
        // Subagent progress is driven by RoundObserver; still log stream errors.
        if let RuntimeEvent::StreamError(msg) = event {
            tracing::warn!(error = %msg, "Subagent stream/runtime error");
        }
    }
}

struct SubagentSynthesis {
    coordinator: Arc<AgentCoordinator>,
    context: AgentExecutionContext,
    synthesized: Mutex<HashSet<String>>,
    is_non_root: bool,
    /// Interception point 2: fallback configuration for model-unavailable
    /// runtime failures. `settings` selects the fallback model;
    /// `transcript_store` recovers the failed child's original `user_prompt`;
    /// `tool_registry` rebuilds the allowed tool set for the re-dispatch.
    settings: Arc<crate::config::Settings>,
    transcript_store: Option<Arc<crate::transcript::SubagentTranscriptStore>>,
    tool_registry: Arc<ToolRegistry>,
}

#[async_trait]
impl SynthesisPort for SubagentSynthesis {
    async fn on_candidate_final(&self, _candidate: &str) -> Result<Option<String>, RuntimeError> {
        if !self.is_non_root {
            // Root (main agent): await in-flight subagents and synthesize their
            // results before finalizing. Without this, the main agent's final
            // answer ignored subagent work entirely -- the task tool returns an
            // immediate ack and root previously short-circuited to `None`, so
            // subagent changes were invisible until a later continuation turn
            // (and dropped entirely if a generation reset intervened).
            return self.synthesize_root_children().await;
        }

        let child_results = self
            .coordinator
            .collect_children_for_synthesis(&self.context)
            .await
            .map_err(|e: CoordinatorError| {
                RuntimeError::Stream(format!("subagent lifecycle coordination failed: {e}"))
            })?;

        // Interception point 2: runtime model-unavailable fallback. For each
        // failed child with `subagent_model_unavailable`, attempt to re-dispatch
        // with a configured fallback model. Replaces the ChildResult in-place on
        // success; leaves it untouched on failure (degrades to parent model).
        let child_results = self.apply_runtime_fallback(child_results).await;

        let fresh: Vec<ChildResult> = {
            let synthesized = self.synthesized.lock().expect("lock poisoned: synthesized");
            child_results
                .iter()
                .filter(|r| !synthesized.contains(r.child_id.as_str()))
                .cloned()
                .collect()
        };

        if !fresh.is_empty() {
            {
                let mut synthesized = self.synthesized.lock().expect("lock poisoned: synthesized");
                for r in &fresh {
                    synthesized.insert(r.child_id.as_str().to_string());
                }
            }
            return Ok(Some(format_child_result_batch(&fresh)));
        }

        self.coordinator
            .begin_finalizing(&self.context)
            .await
            .map_err(|e: CoordinatorError| {
                RuntimeError::Stream(format!("subagent lifecycle coordination failed: {e}"))
            })?;
        Ok(None)
    }
}

impl SubagentSynthesis {
    /// Root-specific synthesis: await all live direct children (so in-flight
    /// subagents complete before the main agent finalizes), claim the ready
    /// root-direct task group for this turn, and return any
    /// not-yet-synthesized results.
    ///
    /// Mirrors the non-root `collect_children_for_synthesis` flow but uses
    /// `claim_ready_root_group` instead of the owner-group transition path,
    /// because the persistent root has no terminal state to transition through.
    ///
    /// Blocking the main agent's final answer until spawned subagents finish
    /// is the desired behavior: the main agent delegated work and should
    /// incorporate it before delivering. Subagents carry their own timeouts,
    /// so this cannot hang indefinitely.
    async fn synthesize_root_children(&self) -> Result<Option<String>, RuntimeError> {
        // Guard: only a true root (parent_id=None AND depth==0) may claim the
        // root-direct task group. A ghost fallback context (parent_id=None but
        // depth>0) is NOT a real root -- claiming would return NotVisible and
        // silently abort the subagent. Short-circuit to Ok(None) instead so the
        // ghost leaf completes naturally without synthesis.
        //
        // This fixes the regression where ghost fallback subagents spawned by
        // `prepare_structural_fallback` (parent_id=None, depth=caller.depth>0)
        // entered this path because `is_non_root = parent_id.is_some()` was
        // false, but `is_root = parent_id.is_none() && depth==0` was also false.
        if self.context.parent_id.is_none() && self.context.depth > 0 {
            tracing::warn!(
                agent_id = %self.context.agent_id,
                depth = self.context.depth,
                "synthesize_root_children: ghost fallback context (parent_id=None, \
                 depth>0) is not a real root; skipping root synthesis to avoid \
                 NotVisible error"
            );
            return Ok(None);
        }

        // Join all live direct children. Awaits each child's terminal state
        // (natural completion, external `finish_child`, or subagent timeout).
        let joined = self
            .coordinator
            .join_children(&self.context, JoinPolicy::BestEffort)
            .await
            .map_err(|e: CoordinatorError| {
                RuntimeError::Stream(format!("root subagent join failed: {e}"))
            })?;

        // Claim the ready root-direct task group for the current generation.
        // The group's recorded results carry richer, subagent-provided
        // summaries than the terminal-derived join results, so prefer them
        // when a delivery is available; fall back to `joined` otherwise
        // (e.g. a child whose group was already claimed in a prior round, or
        // a record_result that has not landed yet despite the terminal).
        let generation = self
            .coordinator
            .current_generation(&self.context.session_id)
            .await;
        let delivery = self
            .coordinator
            .claim_ready_root_group(&self.context, generation)
            .await
            .map_err(|e: CoordinatorError| {
                RuntimeError::Stream(format!("root subagent claim failed: {e}"))
            })?;

        let results = match delivery {
            Some(d) if !d.results.is_empty() => d.results,
            _ => joined,
        };

        let fresh: Vec<ChildResult> = {
            let synthesized = self.synthesized.lock().expect("lock poisoned: synthesized");
            results
                .iter()
                .filter(|r| !synthesized.contains(r.child_id.as_str()))
                .cloned()
                .collect()
        };

        if !fresh.is_empty() {
            {
                let mut synthesized = self.synthesized.lock().expect("lock poisoned: synthesized");
                for r in &fresh {
                    synthesized.insert(r.child_id.as_str().to_string());
                }
            }
            return Ok(Some(format_child_result_batch(&fresh)));
        }

        Ok(None)
    }
}

impl SubagentSynthesis {
    /// Interception point 2: for each failed child whose `error_code` is
    /// `subagent_model_unavailable`, attempt to re-dispatch it with a fallback
    /// model. On success the `ChildResult` is replaced with a `Completed` result;
    /// on failure (no fallback model, no transcript prompt, fallback execution
    /// error, or single-shot already used) the original failed result is kept so
    /// the parent model decides. Root callers skip fallback (Comet isolation).
    async fn apply_runtime_fallback(&self, results: Vec<ChildResult>) -> Vec<ChildResult> {
        use crate::agent::fallback::{
            fallback_eligible_from_child_result, is_root_caller, FallbackKind,
        };

        if is_root_caller(&self.context) {
            return results;
        }

        let mut out = Vec::with_capacity(results.len());
        for r in results {
            let eligible = fallback_eligible_from_child_result(&r);
            if eligible != Some(FallbackKind::ModelUnavailable) {
                out.push(r);
                continue;
            }

            let child_id_str = r.child_id.as_str().to_string();
            if self.coordinator.fallback_already_used(&child_id_str).await {
                tracing::warn!(
                    fallback = "interception2",
                    child_id = %child_id_str,
                    "Fallback already used; skipping (single-shot)"
                );
                out.push(r);
                continue;
            }

            // Mark fallback used BEFORE the attempt (mirrors interception 1).
            // `on_candidate_final` runs every synthesis round, so marking only
            // on success would let a failed fallback re-attempt every round --
            // violating the single-shot, non-recursive constraint.
            self.coordinator.mark_fallback_used(&child_id_str).await;

            match self.attempt_model_fallback(&r).await {
                Ok(new_result) => out.push(new_result),
                Err(reason) => {
                    tracing::warn!(
                        fallback = "interception2",
                        child_id = %child_id_str,
                        reason = %reason,
                        "Model fallback failed; degrading to parent model"
                    );
                    out.push(r);
                }
            }
        }
        out
    }

    /// Re-dispatch a failed child with a fallback model. Returns a replacement
    /// `ChildResult` on success, or an error reason string on failure.
    async fn attempt_model_fallback(&self, failed: &ChildResult) -> Result<ChildResult, String> {
        // 1. Select fallback model (first entry != failed child's model).
        let failed_model = &self.settings.models.main.name;
        let fallback_model = self
            .settings
            .select_fallback_model(failed_model)
            .ok_or_else(|| "no fallback model configured".to_string())?;

        tracing::info!(
            fallback = "interception2",
            child_id = %failed.child_id.as_str(),
            failed_model = %failed_model,
            fallback_model = %fallback_model,
            "Re-dispatching child with fallback model"
        );

        // 2. Recover the original prompt from the transcript store.
        let transcript_store = self
            .transcript_store
            .as_ref()
            .ok_or_else(|| "no transcript store available".to_string())?;
        let transcript = transcript_store
            .get_by_id(failed.child_id.as_str())
            .map_err(|e| format!("transcript read failed: {e}"))?
            .ok_or_else(|| "transcript not found for child".to_string())?;

        let user_prompt = transcript.user_prompt.clone();
        let system_prompt = transcript.system_prompt.clone().unwrap_or_default();

        // 3. Build fallback api_client (swap model name, reuse original endpoint).
        let fallback_settings = self.settings.fallback_model_settings(fallback_model);
        let api_client = ApiClient::new(fallback_settings);

        // 4. Allowed tools: leaf set only (no nested spawn from a ghost agent).
        let tool_registry = Arc::clone(&self.tool_registry);
        let allowed_tools: Vec<String> = tool_registry
            .list()
            .iter()
            .map(|t| t.name().to_string())
            .filter(|name| name != "task" && name != "delegate")
            .collect();

        // 5. Synthesize a leaf context. parent_id=None so SubagentSynthesis
        // skips collect_children_for_synthesis (ghost is not in scopes;
        // non-root would hit NotVisible via active_owner_status).
        let child_context = AgentExecutionContext {
            agent_id: AgentId::new(uuid::Uuid::new_v4().to_string()),
            parent_id: None,
            session_id: self.context.session_id.clone(),
            depth: self.context.depth,
            cancellation: self.context.cancellation.child_token(),
        };

        let timeout_secs = self.settings.agent.subagent.timeout_secs;
        let workdir: Option<std::path::PathBuf> = Some(self.settings.storage.working_dir.clone());
        let permission = SubagentPermissionContext::headless(
            self.settings.storage.working_dir.clone(),
            child_context.agent_id.as_str(),
        );

        // 6. Re-dispatch with the fallback model.
        let result = run_subagent_loop_with_permissions(
            &api_client,
            Arc::clone(&tool_registry),
            &child_context,
            Arc::clone(&self.coordinator),
            &system_prompt,
            &user_prompt,
            &allowed_tools,
            self.settings.agent.subagent.max_rounds.unwrap_or(100),
            timeout_secs,
            None,
            None,
            workdir,
            permission,
            Arc::clone(&self.settings),
            self.transcript_store.clone(),
            None, // interception-2 ghost has no root turn id
        )
        .await;

        match result {
            Ok(summary) => Ok(ChildResult {
                child_id: failed.child_id.clone(),
                status: ChildTerminalStatus::Completed,
                summary: summary.chars().take(500).collect(),
                error_code: None,
                partial_result: None,
            }),
            Err(e) => Err(format!("fallback execution failed: {}", e.full_message())),
        }
    }
}

struct SubagentObserver {
    on_progress: Option<ProgressCallback>,
    trace_id: u64,
    max_rounds: usize,
    start: Instant,
    started_at_ms: i64,
    token_budget_k: Option<u64>,
    context_char_limit: usize,
    action_log: Mutex<Vec<SubagentEvent>>,
    text_snapshot: Mutex<Option<String>>,
    current_params: Mutex<Option<String>>,
    cumulative_tokens: Mutex<usize>,
    /// Shared with GuardingToolPort — drained into action_log on emit.
    permission_events: Arc<Mutex<Vec<(String, String)>>>,
}

impl SubagentObserver {
    fn drain_permission_events_into_log(&self) {
        let pending = {
            let mut log = self
                .permission_events
                .lock()
                .expect("lock poisoned: permission_events");
            std::mem::take(&mut *log)
        };
        if pending.is_empty() {
            return;
        }
        let mut action_log = self.action_log.lock().expect("lock poisoned: action_log");
        let elapsed_ms = self.start.elapsed().as_millis() as u64;
        for (kind, detail) in pending {
            action_log.push(SubagentEvent {
                event_type: SubagentEventType::Permission { kind, detail },
                elapsed_ms,
            });
        }
    }

    fn emit(
        &self,
        status: SubagentStatus,
        round: Option<usize>,
        current_tool: Option<String>,
        error_msg: Option<String>,
        messages: Vec<ChatMessage>,
    ) {
        // Fold any permission lifecycle events into the action log first.
        self.drain_permission_events_into_log();

        let Some(ref cb) = self.on_progress else {
            return;
        };
        let elapsed = self.start.elapsed();
        let is_terminal = matches!(
            status,
            SubagentStatus::Completed | SubagentStatus::Failed | SubagentStatus::Cancelled
        );
        let snapshot = self
            .text_snapshot
            .lock()
            .expect("lock poisoned: text_snapshot")
            .clone();
        let metadata = if is_terminal || error_msg.is_some() {
            Some(SubagentMetadata {
                token_count: Some(
                    *self
                        .cumulative_tokens
                        .lock()
                        .expect("lock poisoned: cumulative_tokens"),
                ),
                error: error_msg.clone(),
                depends_on: vec![],
            })
        } else {
            None
        };
        let action_log_snapshot = self
            .action_log
            .lock()
            .expect("lock poisoned: action_log")
            .clone();
        let error_details = error_msg.as_ref().map(|msg| {
            let diag = build_failure_diagnostics(
                msg,
                status.clone(),
                &action_log_snapshot,
                snapshot.as_deref(),
                self.context_char_limit,
            );
            ErrorInfo {
                error_type: ErrorType::Unknown,
                message: msg.clone(),
                last_tool: current_tool.clone(),
                last_params: self
                    .current_params
                    .lock()
                    .expect("lock poisoned: current_params")
                    .clone(),
                round: round.unwrap_or(0) as u32,
                retryable: true,
                root_cause: diag.root_cause,
                failed_tool_sequence: diag.failed_tool_sequence,
                failed_round_context: diag.failed_round_context,
                retry_history: diag.retry_history,
            }
        });
        cb(SubagentProgress {
            node_id: self.trace_id.to_string(),
            parent_id: None,
            label: String::new(),
            status,
            round,
            max_rounds: Some(self.max_rounds),
            current_tool,
            current_params: self
                .current_params
                .lock()
                .expect("lock poisoned: current_params")
                .clone(),
            action_log: action_log_snapshot.clone(),
            text_snapshot: if is_terminal { None } else { snapshot },
            started_at: self.started_at_ms,
            elapsed_ms: elapsed.as_millis() as u64,
            metadata,
            progress_delta: None,
            token_budget_k: self.token_budget_k,
            cumulative_tokens: *self
                .cumulative_tokens
                .lock()
                .expect("lock poisoned: cumulative_tokens") as u64,
            error_details,
            events: action_log_snapshot,
            messages,
        });
    }
}

impl RoundObserver for SubagentObserver {
    fn on_round_start(&self, round: usize, messages: &[ChatMessage]) {
        // Capture latest assistant text as snapshot when available.
        if let Some(last_asst) = messages.iter().rev().find(|m| m.role == "assistant") {
            if let Some(content) = last_asst.content.as_deref().map(str::trim) {
                if !content.is_empty() {
                    *self
                        .text_snapshot
                        .lock()
                        .expect("lock poisoned: text_snapshot") = Some(content.to_string());
                    self.action_log
                        .lock()
                        .expect("lock poisoned: action_log")
                        .push(SubagentEvent {
                            event_type: SubagentEventType::Thought {
                                text: content.to_string(),
                            },
                            elapsed_ms: self.start.elapsed().as_millis() as u64,
                        });
                }
            }
        }
        self.emit(
            SubagentStatus::Running,
            Some(round),
            None,
            None,
            messages.to_vec(),
        );
    }

    fn on_usage(&self, total_tokens: usize) {
        *self
            .cumulative_tokens
            .lock()
            .expect("lock poisoned: cumulative_tokens") += total_tokens;
    }

    fn on_tool_start(&self, round: usize, tool_name: &str, messages: &[ChatMessage]) {
        // Best-effort params from last assistant tool_calls if present.
        let params = messages
            .iter()
            .rev()
            .find(|m| m.role == "assistant")
            .and_then(|m| m.tool_calls.as_ref())
            .and_then(|tcs| tcs.iter().find(|tc| tc.function.name == tool_name))
            .and_then(|tc| {
                serde_json::from_str::<serde_json::Value>(&tc.function.arguments)
                    .ok()
                    .map(|args| extract_params_summary(tool_name, &args))
            })
            .unwrap_or_default();
        *self
            .current_params
            .lock()
            .expect("lock poisoned: current_params") = Some(params.clone());
        self.action_log
            .lock()
            .expect("lock poisoned: action_log")
            .push(SubagentEvent {
                event_type: SubagentEventType::Action {
                    tool_name: tool_name.to_string(),
                    params_summary: params,
                },
                elapsed_ms: self.start.elapsed().as_millis() as u64,
            });
        self.emit(
            SubagentStatus::Running,
            Some(round),
            Some(tool_name.to_string()),
            None,
            messages.to_vec(),
        );
    }

    fn on_completed(&self, round: usize, messages: &[ChatMessage]) {
        let summary = messages
            .iter()
            .rev()
            .find(|m| m.role == "assistant")
            .and_then(|m| m.content.clone())
            .unwrap_or_default();
        self.action_log
            .lock()
            .expect("lock poisoned: action_log")
            .push(SubagentEvent {
                event_type: SubagentEventType::Completion {
                    status: "completed".to_string(),
                    summary: Some(summary),
                },
                elapsed_ms: self.start.elapsed().as_millis() as u64,
            });
        self.emit(
            SubagentStatus::Completed,
            Some(round),
            None,
            None,
            messages.to_vec(),
        );
    }

    fn on_failed(&self, round: usize, error: &str, messages: &[ChatMessage]) {
        self.action_log
            .lock()
            .expect("lock poisoned: action_log")
            .push(SubagentEvent {
                event_type: SubagentEventType::Error {
                    message: error.to_string(),
                    error_type: ErrorType::Unknown,
                },
                elapsed_ms: self.start.elapsed().as_millis() as u64,
            });
        self.emit(
            SubagentStatus::Failed,
            Some(round),
            None,
            Some(error.to_string()),
            messages.to_vec(),
        );
    }
}

// ── Public entry point ──────────────────────────────────────────────────────

// ── Inbox (s09 mailbox drain) ────────────────────────────────────────────────

/// [`InboxPort`] backed by this subagent's JSONL mailbox.
///
/// Path: `.team/inbox/{agent_id}.jsonl` under the current working directory
/// (matches `TeamManager`'s project_root convention). Each round, pending
/// peer messages are drained and folded into a single `<team-inbox>` system
/// message. An empty inbox returns `None` (no history pollution).
///
/// s10 protocol handling:
/// - `ShutdownRequest` -> cancels the subagent's `CancellationToken` (cooperative
///   shutdown; the outer `run_subagent_loop` select observes it next).
/// - `ApprovalResponse` -> delivered to the matching pending `request_approval`
///   waiter via a oneshot, so the requesting tool unblocks.
struct MailboxInbox {
    mailbox: Mailbox,
    name: String,
    cancellation: tokio_util::sync::CancellationToken,
    /// request_id -> pending approval waiter. Shared so the `request_approval`
    /// tool can register a waiter and this drain can resolve it.
    pending_approvals: Arc<Mutex<HashMap<String, tokio::sync::oneshot::Sender<bool>>>>,
}

impl MailboxInbox {
    /// Open (or create) the inbox for `agent_id`. Best-effort: if the path
    /// cannot be resolved, returns None and the subagent runs without an inbox.
    fn for_agent(
        agent_id: &str,
        cancellation: tokio_util::sync::CancellationToken,
        pending_approvals: Arc<Mutex<HashMap<String, tokio::sync::oneshot::Sender<bool>>>>,
    ) -> Option<Self> {
        let cwd = std::env::current_dir().ok()?;
        let inbox_dir = cwd.join(".team").join("inbox");
        let path = inbox_dir.join(format!("{}.jsonl", sanitize_mailbox_name(agent_id)));
        Some(Self {
            mailbox: Mailbox::new(path),
            name: agent_id.to_string(),
            cancellation,
            pending_approvals,
        })
    }
}

#[async_trait]
impl InboxPort for MailboxInbox {
    async fn drain(&self) -> Option<String> {
        let messages = match self.mailbox.receive_all().await {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!(error = %e, agent = %self.name, "mailbox drain failed");
                return None;
            }
        };
        if messages.is_empty() {
            return None;
        }

        let mut body: Vec<String> = Vec::new();
        let mut got_shutdown = false;
        for m in &messages {
            match m {
                TeamMessage::Message { from, content, .. } => {
                    body.push(format!("[from {from}] {content}"));
                }
                TeamMessage::Broadcast { from, content, .. } => {
                    body.push(format!("[broadcast from {from}] {content}"));
                }
                TeamMessage::ShutdownRequest { from, request_id } => {
                    body.push(format!(
                        "[shutdown request from {from} id={request_id}] - shutting down"
                    ));
                    got_shutdown = true;
                }
                TeamMessage::ShutdownResponse {
                    from,
                    request_id,
                    approve,
                } => {
                    body.push(format!(
                        "[shutdown response from {from} id={request_id} approve={approve}]"
                    ));
                }
                TeamMessage::ApprovalRequest {
                    from,
                    request_id,
                    kind,
                    payload,
                    tool,
                    policy_reason,
                    session_rule,
                    ..
                } => {
                    let structured = match (
                        tool.as_deref(),
                        policy_reason.as_deref(),
                        session_rule.as_deref(),
                    ) {
                        (Some(t), Some(reason), Some(rule)) => {
                            format!(" tool={t} reason={reason} rule={rule}")
                        }
                        _ => String::new(),
                    };
                    body.push(format!(
                        "[approval request from {from} id={request_id} kind={kind}{structured}] {payload}"
                    ));
                }
                TeamMessage::ApprovalResponse {
                    from,
                    request_id,
                    approve,
                    ..
                } => {
                    // Deliver to the waiting request_approval tool, if any.
                    if let Some(tx) = self
                        .pending_approvals
                        .lock()
                        .expect("lock poisoned: pending_approvals")
                        .remove(request_id)
                    {
                        let _ = tx.send(*approve);
                    }
                    body.push(format!(
                        "[approval response from {from} id={request_id} approve={approve}]"
                    ));
                }
            }
        }

        // Cooperative shutdown: cancel the subagent. The outer select in
        // run_subagent_loop observes the cancellation and returns Cancelled.
        if got_shutdown {
            self.cancellation.cancel();
        }

        Some(format!(
            "<team-inbox>
You received {} message(s) from teammates:
{}
</team-inbox>",
            body.len(),
            body.join(
                "
"
            )
        ))
    }
}

/// Restrict mailbox file names to a safe subset (agent ids are UUIDs, but be defensive).
fn sanitize_mailbox_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Build a trace sink from settings, or `None` when trace streaming is off
/// (or daemon-only, which has no file sink in Task 3.2). The sink writes to
/// `<trace_dir>/<session_id>.jsonl`, aggregating all subagents in the session.
fn build_trace_sink(settings: &crate::config::Settings, session_id: &str) -> Option<TraceSink> {
    let trace = &settings.agent.subagent.trace;
    TraceSink::for_mode(trace.sink, trace.dir.as_deref(), session_id)
}

/// Run a subagent with an isolated agent loop via the shared runtime, with an
/// explicit permission context (shared session_rules / optional approval
/// bridge / guardian).
///
/// `origin_turn_id`, when set, folds subagent file edits into the parent
/// turn's checkpoint snapshot (no independent subagent checkpoint).
#[allow(clippy::too_many_arguments)]
pub async fn run_subagent_loop_with_permissions(
    api_client: &ApiClient,
    tool_registry: Arc<ToolRegistry>,
    context: &AgentExecutionContext,
    coordinator: Arc<AgentCoordinator>,
    system_prompt: &str,
    user_prompt: &str,
    allowed_tools: &[String],
    max_rounds: usize,
    timeout_secs: u64,
    on_progress: Option<ProgressCallback>,
    token_budget_k: Option<u64>,
    workdir: Option<std::path::PathBuf>,
    permission: SubagentPermissionContext,
    settings: Arc<crate::config::Settings>,
    transcript_store: Option<Arc<crate::transcript::SubagentTranscriptStore>>,
    origin_turn_id: Option<String>,
) -> Result<String, SubagentError> {
    let timeout_duration = Duration::from_secs(timeout_secs);
    static SUBAGENT_COUNTER: AtomicU64 = AtomicU64::new(0);
    let trace_id = SUBAGENT_COUNTER.fetch_add(1, Ordering::Relaxed);
    let denial_log = Arc::clone(&permission.denial_log);
    let event_log = Arc::clone(&permission.event_log);

    tracing::info!(
        prompt_len = user_prompt.len(),
        tool_count = allowed_tools.len(),
        max_rounds = max_rounds,
        timeout_secs = timeout_secs,
        trace_id = trace_id,
        "Subagent: starting agent loop (shared runtime, guarding tools)"
    );

    let start = Instant::now();
    let started_at_ms = chrono::Utc::now().timestamp_millis();

    let system_messages = vec![ChatMessage::system(system_prompt)];
    let seed = vec![ChatMessage::user(user_prompt)];
    let history = MutexHistoryStore::new(Arc::new(tokio::sync::Mutex::new(seed)));

    let llm = ApiLlmPort::new(api_client.clone());
    let allowed: HashSet<String> = allowed_tools.iter().cloned().collect();
    // Prefer the explicit root turn id; fall back to a fresh id only so the
    // shared loop still stamps ToolRequest.turn_id. Capture itself keys off
    // GuardingToolPort.origin_turn_id (root), not the loop's own id.
    let loop_turn_id = origin_turn_id
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let tools = GuardingToolPort::new(&tool_registry, context, allowed, workdir, permission)
        .with_origin_turn_id(origin_turn_id);
    let events = NullEventSink;

    // Resolve the context window from the subagent's effective model (small
    // endpoint if configured, else main) before `settings` is moved into the
    // synthesis port below.
    let context_window = resolve_context_window(
        settings
            .models
            .small
            .as_ref()
            .unwrap_or(&settings.models.main),
        settings.models.context_window,
    );

    let is_non_root = context.parent_id.is_some();

    // s04/s09: optional trace sink (subagent.trace.sink). Built before
    // `settings` is moved into the synthesis port. The sink's callback is
    // composed with the caller's `on_progress` so trace emission is
    // transparent to the loop; file/dir permissions and redaction are handled
    // inside TraceSink, and the writer task never blocks the agent loop.
    let trace_sink = build_trace_sink(&settings, context.session_id.as_str());
    let on_progress = compose_progress_callback(on_progress, trace_sink.as_ref());
    let context_char_limit = settings.agent.subagent.trace.context_char_limit;

    let synthesis = SubagentSynthesis {
        coordinator,
        context: context.clone(),
        synthesized: Mutex::new(HashSet::new()),
        is_non_root,
        settings,
        transcript_store,
        tool_registry: Arc::clone(&tool_registry),
    };

    let observer = SubagentObserver {
        on_progress: on_progress.clone(),
        trace_id,
        max_rounds,
        start,
        started_at_ms,
        token_budget_k,
        context_char_limit,
        action_log: Mutex::new(Vec::new()),
        text_snapshot: Mutex::new(None),
        current_params: Mutex::new(None),
        cumulative_tokens: Mutex::new(0),
        permission_events: event_log,
    };

    // Initial Running emit.
    observer.emit(
        SubagentStatus::Running,
        Some(0),
        None,
        None,
        history.get().await,
    );

    let config = RuntimeConfig {
        max_rounds,
        plan_mode: false,
        subagent_timeout_secs: timeout_secs,
        context_window,
        max_tokens: 4096,
        session_id: context.session_id.as_str().to_string(),
        turn_id: Some(loop_turn_id),
        agent_generation: 0,
        stream_max_retries: 0,
    };

    // s09/s10: open this subagent's team mailbox (best-effort). The shared
    // pending-approvals map is also held by the `request_approval` tool so
    // ApprovalResponse messages drain-observed here resolve its waiter.
    let pending_approvals: Arc<Mutex<HashMap<String, tokio::sync::oneshot::Sender<bool>>>> =
        approval_registry::register_agent(context.agent_id.as_str());
    let inbox = MailboxInbox::for_agent(
        context.agent_id.as_str(),
        context.cancellation.clone(),
        pending_approvals.clone(),
    );
    let mut state = LoopTurnState::default();
    let mut stuck = StuckDetector::new();

    let loop_future = run_agent_loop(RunLoopArgs {
        llm: &llm,
        tools: &tools,
        events: &events,
        history: &history,
        config: &config,
        state: &mut state,
        stream_style: StreamStyle::subagent(),
        hooks: LoopHooks {
            compactor: None,
            interaction: None,
            planner: None,
            stuck_detector: Some(&mut stuck),
            token_counter: None,
            synthesis: Some(&synthesis),
            observer: Some(&observer),
            task_progress: None,
            inbox: inbox.as_ref().map(|i| i as &dyn InboxPort),
            session: None,
        },
        system_messages: &system_messages,
    });

    let timeout_future = tokio::time::timeout(timeout_duration, loop_future);
    tokio::pin!(timeout_future);
    let cancelled = context.cancellation.cancelled();
    tokio::pin!(cancelled);

    let result = tokio::select! {
        biased;
        _ = &mut cancelled => {
            observer.action_log.lock().expect("lock poisoned: action_log").push(SubagentEvent {
                event_type: SubagentEventType::Error {
                    message: "Subagent cancelled by parent scope".to_string(),
                    error_type: ErrorType::Cancelled,
                },
                elapsed_ms: start.elapsed().as_millis() as u64,
            });
            let msgs = history.get().await;
            observer.emit(
                SubagentStatus::Cancelled,
                None,
                None,
                Some("Subagent cancelled by parent scope".to_string()),
                msgs,
            );
            tracing::info!(
                trace_id = trace_id,
                elapsed_secs = start.elapsed().as_secs(),
                "Subagent: cancelled by parent scope"
            );
            Err(SubagentError {
                message: "Subagent cancelled by parent scope".to_string(),
                error_type: ErrorType::Cancelled,
                partial_result: observer.text_snapshot.lock().expect("lock poisoned: text_snapshot").clone(),
            })
        }
        result = &mut timeout_future => match result {
            Ok(Ok(text)) => {
                tracing::info!(
                    trace_id = trace_id,
                    elapsed_secs = start.elapsed().as_secs(),
                    "Subagent: completed successfully"
                );
                let reasons = denial_log
                    .lock()
                    .expect("lock poisoned: denial_log")
                    .clone();
                let suffix = format_permission_summary(&reasons);
                if suffix.is_empty() {
                    Ok(text)
                } else if text.trim().is_empty() {
                    Ok(suffix)
                } else {
                    Ok(format!("{text}\n\n{suffix}"))
                }
            }
            Ok(Err(e)) => {
                let (error_type, message) = match &e {
                    RuntimeError::MaxRoundsExceeded { .. } => (
                        ErrorType::Stuck {
                            reason: "exceeded maximum rounds".to_string(),
                        },
                        e.to_string(),
                    ),
                    RuntimeError::StreamTimeout(_) => (ErrorType::Timeout, e.to_string()),
                    RuntimeError::Stream(msg) => (classify_stream_error(msg), msg.clone()),
                    other => (ErrorType::Unknown, other.to_string()),
                };
                Err(SubagentError {
                    message,
                    error_type,
                    partial_result: observer.text_snapshot.lock().expect("lock poisoned: text_snapshot").clone(),
                })
            }
            Err(_elapsed) => {
                observer.action_log.lock().expect("lock poisoned: action_log").push(SubagentEvent {
                    event_type: SubagentEventType::Error {
                        message: format!("Timed out after {} seconds", timeout_duration.as_secs()),
                        error_type: ErrorType::Timeout,
                    },
                    elapsed_ms: start.elapsed().as_millis() as u64,
                });
                let msgs = history.get().await;
                observer.emit(
                    SubagentStatus::Failed,
                    None,
                    None,
                    Some(format!(
                        "Timed out after {} seconds",
                        timeout_duration.as_secs()
                    )),
                    msgs,
                );
                tracing::error!(
                    trace_id = trace_id,
                    timeout_secs = timeout_duration.as_secs(),
                    elapsed_secs = start.elapsed().as_secs(),
                    "Subagent: timed out"
                );
                Err(SubagentError {
                    message: format!(
                        "Subagent timed out after {} seconds",
                        timeout_duration.as_secs()
                    ),
                    error_type: ErrorType::Timeout,
                    partial_result: observer.text_snapshot.lock().expect("lock poisoned: text_snapshot").clone(),
                })
            }
        },
    };

    if let Some(sink) = trace_sink {
        if let Err(e) = sink.shutdown().await {
            tracing::warn!(target: "wgenty::trace_sink", error = %e, "trace sink shutdown failed");
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::SessionId;

    use crate::teams::failure_diagnostics::FailureRootCause;
    use crate::teams::failure_diagnostics::RetryOutcome;

    fn ev(event_type: SubagentEventType, elapsed_ms: u64) -> SubagentEvent {
        SubagentEvent {
            event_type,
            elapsed_ms,
        }
    }

    #[test]
    fn extract_signals_user_cancelled_from_status() {
        assert!(extract_failure_signals("err", SubagentStatus::Cancelled, &[]).user_cancelled);
        assert!(extract_failure_signals("err", SubagentStatus::Cancelling, &[]).user_cancelled);
        assert!(!extract_failure_signals("err", SubagentStatus::Failed, &[]).user_cancelled);
    }

    #[test]
    fn extract_signals_guardian_rejected_from_permission_event() {
        let log = vec![ev(
            SubagentEventType::Permission {
                kind: "permission_denied".into(),
                detail: "rm -rf blocked".into(),
            },
            100,
        )];
        let signals = extract_failure_signals("tool failed", SubagentStatus::Failed, &log);
        assert_eq!(
            signals.guardian_rejected_reason.as_deref(),
            Some("rm -rf blocked")
        );
    }

    #[test]
    fn extract_signals_guardian_rejected_picks_most_recent() {
        let log = vec![
            ev(
                SubagentEventType::Permission {
                    kind: "permission_denied".into(),
                    detail: "first".into(),
                },
                10,
            ),
            ev(
                SubagentEventType::Permission {
                    kind: "permission_denied".into(),
                    detail: "second".into(),
                },
                20,
            ),
        ];
        let signals = extract_failure_signals("err", SubagentStatus::Failed, &log);
        assert_eq!(signals.guardian_rejected_reason.as_deref(), Some("second"));
    }

    #[test]
    fn extract_signals_sandbox_and_panic_from_msg() {
        assert!(
            extract_failure_signals("sandbox execution denied", SubagentStatus::Failed, &[])
                .sandbox_failed
        );
        assert!(
            extract_failure_signals("tool panicked: unwind", SubagentStatus::Failed, &[])
                .tool_panic
        );
        let neutral = extract_failure_signals("plain error", SubagentStatus::Failed, &[]);
        assert!(!neutral.sandbox_failed && !neutral.tool_panic);
    }

    #[test]
    fn failed_tool_sequence_takes_last_round_actions() {
        let log = vec![
            ev(
                SubagentEventType::Thought {
                    text: "round1".into(),
                },
                0,
            ),
            ev(
                SubagentEventType::Action {
                    tool_name: "grep".into(),
                    params_summary: "pattern=foo".into(),
                },
                10,
            ),
            ev(
                SubagentEventType::ToolResult {
                    tool_name: "grep".into(),
                    success: true,
                    summary: "3 matches".into(),
                },
                20,
            ),
            ev(
                SubagentEventType::Thought {
                    text: "round2".into(),
                },
                30,
            ),
            ev(
                SubagentEventType::Action {
                    tool_name: "file_write".into(),
                    params_summary: "path=/x".into(),
                },
                40,
            ),
            ev(
                SubagentEventType::ToolResult {
                    tool_name: "file_write".into(),
                    success: false,
                    summary: "EACCES".into(),
                },
                50,
            ),
        ];
        let seq = build_failed_tool_sequence(&log);
        assert_eq!(seq.len(), 1, "only the last round's action: {seq:?}");
        assert_eq!(seq[0].tool_name, "file_write");
        assert_eq!(seq[0].elapsed_ms, 10, "delta to ToolResult (50-40)");
        assert_eq!(
            seq[0].params_summary,
            serde_json::Value::String("path=/x".into())
        );
    }

    #[test]
    fn failed_tool_sequence_action_without_result_emits_zero_elapsed() {
        let log = vec![
            ev(SubagentEventType::Thought { text: "r".into() }, 0),
            ev(
                SubagentEventType::Action {
                    tool_name: "grep".into(),
                    params_summary: "p".into(),
                },
                10,
            ),
        ];
        let seq = build_failed_tool_sequence(&log);
        assert_eq!(seq.len(), 1);
        assert_eq!(seq[0].elapsed_ms, 0);
    }

    #[test]
    fn failed_tool_sequence_no_thought_takes_all_actions() {
        let log = vec![
            ev(
                SubagentEventType::Action {
                    tool_name: "grep".into(),
                    params_summary: "p".into(),
                },
                5,
            ),
            ev(
                SubagentEventType::ToolResult {
                    tool_name: "grep".into(),
                    success: true,
                    summary: "ok".into(),
                },
                15,
            ),
        ];
        let seq = build_failed_tool_sequence(&log);
        assert_eq!(seq.len(), 1);
        assert_eq!(seq[0].elapsed_ms, 10);
    }

    #[test]
    fn failed_tool_sequence_wraps_params_as_json_string() {
        let log = vec![
            ev(
                SubagentEventType::Action {
                    tool_name: "x".into(),
                    params_summary: "token=secret".into(),
                },
                0,
            ),
            ev(
                SubagentEventType::ToolResult {
                    tool_name: "x".into(),
                    success: true,
                    summary: "ok".into(),
                },
                1,
            ),
        ];
        let seq = build_failed_tool_sequence(&log);
        assert_eq!(
            seq[0].params_summary,
            serde_json::Value::String("token=secret".into())
        );
    }

    #[test]
    fn failed_round_context_uses_last_thought_and_tool_output() {
        let log = vec![
            ev(
                SubagentEventType::Thought {
                    text: "analyzing".into(),
                },
                0,
            ),
            ev(
                SubagentEventType::Action {
                    tool_name: "grep".into(),
                    params_summary: "p".into(),
                },
                10,
            ),
            ev(
                SubagentEventType::ToolResult {
                    tool_name: "grep".into(),
                    success: false,
                    summary: "error output".into(),
                },
                20,
            ),
        ];
        let ctx = build_failed_round_context(&log, None, 2000).expect("context present");
        assert_eq!(ctx.assistant_text, "analyzing");
        assert_eq!(ctx.final_tool_output, "error output");
    }

    #[test]
    fn failed_round_context_truncates_to_char_limit() {
        let long = "a".repeat(500);
        let log = vec![ev(SubagentEventType::Thought { text: long }, 0)];
        let ctx = build_failed_round_context(&log, None, 100).expect("context present");
        assert!(ctx.assistant_text.chars().count() <= 100);
        assert!(ctx.final_tool_output.is_empty());
    }

    #[test]
    fn failed_round_context_none_when_no_data() {
        assert!(build_failed_round_context(&[], None, 2000).is_none());
    }

    #[test]
    fn failed_round_context_falls_back_to_snapshot() {
        let log = vec![ev(
            SubagentEventType::Action {
                tool_name: "x".into(),
                params_summary: "p".into(),
            },
            0,
        )];
        let ctx =
            build_failed_round_context(&log, Some("snapshot text"), 2000).expect("context present");
        assert_eq!(ctx.assistant_text, "snapshot text");
    }

    #[test]
    fn build_diagnostics_cancelled_yields_user_cancelled_root_cause() {
        let diag = build_failure_diagnostics(
            "cancelled by user",
            SubagentStatus::Cancelled,
            &[],
            None,
            2000,
        );
        assert_eq!(diag.root_cause, FailureRootCause::UserCancelled);
    }

    #[test]
    fn build_diagnostics_guardian_rejection() {
        let log = vec![ev(
            SubagentEventType::Permission {
                kind: "permission_denied".into(),
                detail: "dangerous cmd".into(),
            },
            0,
        )];
        let diag =
            build_failure_diagnostics("tool failed", SubagentStatus::Failed, &log, None, 2000);
        assert_eq!(
            diag.root_cause,
            FailureRootCause::GuardianRejected {
                reason: "dangerous cmd".into()
            }
        );
    }

    #[test]
    fn build_diagnostics_sandbox_failure() {
        let diag = build_failure_diagnostics(
            "sandbox execution failed",
            SubagentStatus::Failed,
            &[],
            None,
            2000,
        );
        assert_eq!(diag.root_cause, FailureRootCause::SandboxFailed);
    }

    #[test]
    fn build_diagnostics_tool_panic() {
        let diag =
            build_failure_diagnostics("tool panicked", SubagentStatus::Failed, &[], None, 2000);
        assert_eq!(diag.root_cause, FailureRootCause::ToolPanic);
    }

    #[test]
    fn build_diagnostics_timeout_via_string_fallback() {
        let diag = build_failure_diagnostics(
            "subagent timed out after 600s",
            SubagentStatus::Failed,
            &[],
            None,
            2000,
        );
        assert_eq!(diag.root_cause, FailureRootCause::Timeout);
    }

    #[test]
    fn build_retry_history_empty_when_no_signals() {
        assert!(build_retry_history(&[]).is_empty());
    }

    #[test]
    fn build_retry_history_records_each_attempt() {
        let signals = vec![
            RetrySignal {
                error: "e1".into(),
                root_cause: FailureRootCause::ApiError,
                strategy: "model_fallback".into(),
                outcome: RetryOutcome::Failed,
            },
            RetrySignal {
                error: "e2".into(),
                root_cause: FailureRootCause::ApiError,
                strategy: "model_fallback".into(),
                outcome: RetryOutcome::Failed,
            },
            RetrySignal {
                error: String::new(),
                root_cause: FailureRootCause::Unknown,
                strategy: "model_fallback".into(),
                outcome: RetryOutcome::Succeeded,
            },
        ];
        let history = build_retry_history(&signals);
        assert_eq!(history.len(), 3);
        assert_eq!(history[0].error, "e1");
        assert_eq!(history[0].outcome, RetryOutcome::Failed);
        assert_eq!(history[2].outcome, RetryOutcome::Succeeded);
        assert_eq!(history[2].root_cause, FailureRootCause::Unknown);
    }

    #[test]
    fn build_diagnostics_single_attempt_has_empty_retry_history() {
        // The subagent loop has no in-loop subagent-level retry today (the only
        // re-attempt is the parent-level `attempt_model_fallback` model
        // re-dispatch, which spawns a separate child run). A single-attempt
        // failure therefore yields an empty retry_history.
        let diag = build_failure_diagnostics("err", SubagentStatus::Failed, &[], None, 2000);
        assert!(
            diag.retry_history.is_empty(),
            "no in-loop retry -> empty retry_history"
        );
    }

    #[test]
    fn extract_params_summary_truncates_multibyte_at_char_boundary() {
        let description = format!("{}构suffix", "a".repeat(79));
        let args = serde_json::json!({ "description": description });
        let summary = extract_params_summary("task", &args);

        assert!(
            summary.ends_with('…'),
            "truncated summary should end with ellipsis, got: {summary:?}"
        );
        assert!(!summary.contains('构'));
        assert!(
            !summary.contains('\u{FFFD}'),
            "no replacement char from a split codepoint"
        );
    }

    #[test]
    fn extract_params_summary_preserves_short_multibyte_value() {
        let args = serde_json::json!({ "description": "短任务" });
        let summary = extract_params_summary("task", &args);
        assert_eq!(summary, "短任务");
    }

    #[test]
    fn model_unavailable_maps_to_subagent_model_unavailable_code() {
        let err = SubagentError {
            message: "API error (503): service unavailable".to_string(),
            error_type: ErrorType::ModelUnavailable,
            partial_result: None,
        };
        assert_eq!(err.code(), "subagent_model_unavailable");
    }

    #[test]
    fn classify_stream_error_api_error_is_model_unavailable() {
        assert_eq!(
            classify_stream_error("API error (503): service unavailable"),
            ErrorType::ModelUnavailable
        );
    }

    #[test]
    fn classify_stream_error_connection_is_model_unavailable() {
        assert_eq!(
            classify_stream_error("connection refused by host"),
            ErrorType::ModelUnavailable
        );
    }

    #[test]
    fn classify_stream_error_http_status_is_model_unavailable() {
        assert_eq!(
            classify_stream_error("request failed: HTTP 500 internal server error"),
            ErrorType::ModelUnavailable
        );
    }

    #[test]
    fn classify_stream_error_stuck_remains_stuck() {
        assert_eq!(
            classify_stream_error("subagent stuck in loop"),
            ErrorType::Stuck {
                reason: "subagent stuck in loop".to_string(),
            }
        );
    }

    #[test]
    fn classify_stream_error_other_remains_unknown() {
        assert_eq!(
            classify_stream_error("some unexpected stream error"),
            ErrorType::Unknown
        );
    }

    // ── Ghost guard tests for synthesize_root_children ──────────────────────
    //
    // Regression tests for P0#3: a ghost fallback context (parent_id=None but
    // depth>0) must NOT trigger NotVisible in synthesize_root_children. It
    // should short-circuit to Ok(None). A real root (parent_id=None, depth==0)
    // must still proceed through the normal join/claim path.

    fn make_synthesis(context: AgentExecutionContext) -> SubagentSynthesis {
        let coordinator = Arc::new(AgentCoordinator::new(5, 1));
        let is_non_root = context.parent_id.is_some();
        SubagentSynthesis {
            coordinator,
            context,
            synthesized: Mutex::new(HashSet::new()),
            is_non_root,
            settings: Arc::new(crate::config::Settings::default()),
            transcript_store: None,
            tool_registry: Arc::new(ToolRegistry::new()),
        }
    }

    fn make_ghost_context(depth: usize) -> AgentExecutionContext {
        // Mirrors prepare_structural_fallback (fallback.rs:112-118):
        // parent_id=None, depth=caller.depth (>0 for a non-root caller).
        AgentExecutionContext {
            agent_id: AgentId::new(uuid::Uuid::new_v4().to_string()),
            parent_id: None,
            session_id: SessionId::new("test-ghost-session"),
            depth,
            cancellation: tokio_util::sync::CancellationToken::new(),
        }
    }

    #[tokio::test]
    async fn ghost_context_synthesize_returns_ok_none_not_not_visible() {
        // A ghost fallback context (parent_id=None, depth>0) must short-circuit
        // to Ok(None) instead of triggering NotVisible in claim_ready_root_group.
        let ghost = make_ghost_context(1);
        let synthesis = make_synthesis(ghost);

        let result = synthesis.on_candidate_final("test").await;

        assert!(
            result.is_ok(),
            "ghost context must not error (NotVisible regression); got: {:?}",
            result.err()
        );
        assert_eq!(
            result.unwrap(),
            None,
            "ghost context with no children should return Ok(None)"
        );
    }

    #[tokio::test]
    async fn real_root_synthesize_does_not_short_circuit() {
        // A real root (parent_id=None, depth==0) must NOT short-circuit. With no
        // live children, join_children returns empty and claim returns None,
        // yielding Ok(None) -- but via the normal path, not the ghost guard.
        let root = AgentExecutionContext::root(SessionId::new("test-root-session"));
        let synthesis = make_synthesis(root);

        let result = synthesis.on_candidate_final("test").await;

        // Real root with no children: join returns empty, claim returns None,
        // fresh is empty -> Ok(None). The key assertion is no error.
        assert!(
            result.is_ok(),
            "real root with no children must not error; got: {:?}",
            result.err()
        );
    }

    #[tokio::test]
    async fn ghost_context_at_depth_2_also_short_circuits() {
        // Deeper ghost (depth=2, e.g. nested fallback) must also short-circuit.
        let ghost = make_ghost_context(2);
        let synthesis = make_synthesis(ghost);

        let result = synthesis.on_candidate_final("test").await;

        assert!(
            result.is_ok(),
            "ghost at depth 2 must not error; got: {:?}",
            result.err()
        );
    }
}
