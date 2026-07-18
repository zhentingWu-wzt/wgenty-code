//! Subagent Loop — isolated agent loop for subagent execution.
//!
//! Control flow is [`crate::agent::runtime::run_agent_loop`]. This module
//! provides subagent-specific ports (guarding tools, progress observer,
//! non-root synthesis barrier) and preserves the historical
//! [`run_subagent_loop`] signature for `task` / RLM / run_script callers.

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
    CoordinatorError,
};
use crate::api::{ApiClient, ChatMessage};
use crate::teams::approval_registry;
use crate::teams::guarding_tool_port::{
    format_permission_summary, GuardingToolPort, SubagentPermissionContext,
};
use crate::teams::mailbox::{Mailbox, TeamMessage};
use crate::tools::ToolRegistry;
use crate::utils::stuck_detector::StuckDetector;
use async_trait::async_trait;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

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

/// Structured error returned by [`run_subagent_loop`] when a subagent fails.
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
            return Ok(None);
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

        // 4. Allowed tools: same registry, all non-spawn tools at this depth.
        let tool_registry = Arc::clone(&self.tool_registry);
        let allowed_tools: Vec<String> = tool_registry
            .list()
            .iter()
            .map(|t| t.name().to_string())
            .collect();

        // 5. Synthesize a child context reusing the parent's session/depth.
        let child_context = AgentExecutionContext {
            agent_id: AgentId::new(uuid::Uuid::new_v4().to_string()),
            parent_id: Some(self.context.agent_id.clone()),
            session_id: self.context.session_id.clone(),
            depth: self.context.depth,
            cancellation: self.context.cancellation.clone(),
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
        let error_details = error_msg.as_ref().map(|msg| ErrorInfo {
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
        });
        let action_log_snapshot = self
            .action_log
            .lock()
            .expect("lock poisoned: action_log")
            .clone();
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

/// Run a subagent with an isolated agent loop via the shared runtime.
#[allow(clippy::too_many_arguments)]
pub async fn run_subagent_loop(
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
) -> Result<String, SubagentError> {
    // Headless default: shared workspace policy, no approval bridge (Ask fail closed).
    // Call sites that own a root ToolExecutor should pass a richer context via
    // `run_subagent_loop_with_permissions` once fully wired.
    let permission = SubagentPermissionContext::headless(
        std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")),
        context.agent_id.as_str(),
    );
    // Headless path has no Settings/transcript_store: interception-2 fallback
    // degrades to parent model (no fallback_models configured, no prompt source).
    let settings = Arc::new(crate::config::Settings::default());
    run_subagent_loop_with_permissions(
        api_client,
        tool_registry,
        context,
        coordinator,
        system_prompt,
        user_prompt,
        allowed_tools,
        max_rounds,
        timeout_secs,
        on_progress,
        token_budget_k,
        workdir,
        permission,
        settings,
        None,
    )
    .await
}

/// Same as [`run_subagent_loop`] but with an explicit permission context
/// (shared session_rules / optional approval bridge / guardian).
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

    let seed = vec![
        ChatMessage::system(system_prompt),
        ChatMessage::user(user_prompt),
    ];
    let history = MutexHistoryStore::new(Arc::new(tokio::sync::Mutex::new(seed)));

    let llm = ApiLlmPort::new(api_client.clone());
    let allowed: HashSet<String> = allowed_tools.iter().cloned().collect();
    let tools = GuardingToolPort::new(&tool_registry, context, allowed, workdir, permission);
    let events = NullEventSink;

    let is_non_root = context.parent_id.is_some();
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
        context_window: 200_000,
        max_tokens: 4096,
        session_id: context.session_id.as_str().to_string(),
        turn_id: None,
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
        },
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

    result
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
