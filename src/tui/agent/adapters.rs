//! TUI-side port adapters for the shared agent runtime.
//!
//! Lives under `tui` so `agent::runtime` never depends on TUI or DaemonClient.

use crate::agent::runtime::{
    archive_transcript, assemble_post_compaction_history, parse_compaction_response, Compactor,
    EventSink, HistoryStore, InteractionPort, LlmPort, PlannerPort, RuntimeError, RuntimeEvent,
    ToolPort, ToolRequest, ToolResponse, COMPACTION_SYSTEM_PROMPT,
};
use crate::agent::{StreamEvent, StreamProcessor};
use crate::api::{ApiClient, ChatMessage, ToolDefinition};
use crate::context::MemoryManager;
use crate::runtime::guardian::classify_risk;
use crate::runtime::hooks::HookManager;
use crate::tui::app::{AppEvent, QuestionOption};
use crate::tui::client::DaemonClient;
use async_trait::async_trait;
use bytes::Bytes;
use futures::stream::{BoxStream, StreamExt};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

// ── Event sink ──────────────────────────────────────────────────────────────

pub struct TuiEventSink {
    tx: mpsc::UnboundedSender<AppEvent>,
}

impl TuiEventSink {
    pub fn new(tx: mpsc::UnboundedSender<AppEvent>) -> Self {
        Self { tx }
    }
}

impl EventSink for TuiEventSink {
    fn emit(&self, event: RuntimeEvent) {
        let app_event = match event {
            RuntimeEvent::Connecting {
                attempt,
                max_retries,
            } => AppEvent::Connecting {
                attempt,
                max_retries,
            },
            RuntimeEvent::ContentDelta(text) => AppEvent::ContentDelta(text),
            RuntimeEvent::ReasoningDelta(text) => AppEvent::ReasoningDelta(text),
            RuntimeEvent::PreparingTools => AppEvent::PreparingTools,
            RuntimeEvent::StreamDone { finish_reason } => AppEvent::StreamDone { finish_reason },
            RuntimeEvent::StreamError(msg) => AppEvent::StreamError(msg),
            RuntimeEvent::CompactionStarted => AppEvent::CompactionStarted,
            RuntimeEvent::ContextCompacted { summary_chars } => {
                AppEvent::ContextCompacted { summary_chars }
            }
            RuntimeEvent::ToolStart { name, args } => AppEvent::ToolStart { name, args },
            RuntimeEvent::ToolResult {
                name,
                args,
                content,
            } => AppEvent::ToolResult {
                name,
                args,
                content,
            },
            RuntimeEvent::BackgroundTaskResult(msg) => AppEvent::BackgroundTaskResult(msg),
            RuntimeEvent::PlanUpdate(value) => AppEvent::PlanUpdate(value),
            RuntimeEvent::SaveSession => AppEvent::SaveSession,
        };
        let _ = self.tx.send(app_event);
    }
}

// ── LLM (daemon) ────────────────────────────────────────────────────────────

pub struct DaemonLlmPort {
    client: DaemonClient,
}

impl DaemonLlmPort {
    pub fn new(client: DaemonClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl LlmPort for DaemonLlmPort {
    async fn open_chat_stream(
        &self,
        messages: Vec<ChatMessage>,
        _tools: Option<Vec<ToolDefinition>>,
        max_tokens: Option<usize>,
        plan_mode: Option<bool>,
    ) -> Result<BoxStream<'static, Result<Bytes, RuntimeError>>, RuntimeError> {
        let response = self
            .client
            .chat_stream_with_plan(messages, max_tokens, plan_mode)
            .await
            .map_err(|e| RuntimeError::from_stream_failure(e.to_string()))?;

        let stream = response
            .bytes_stream()
            .map(|item| item.map_err(|e| RuntimeError::from_stream_failure(e.to_string())));
        Ok(Box::pin(stream))
    }
}

// ── Tools (daemon + permission UI + subagent progress poll) ─────────────────

pub struct DaemonToolPort {
    client: DaemonClient,
    event_tx: mpsc::UnboundedSender<AppEvent>,
    hook_manager: Arc<HookManager>,
    session_id: String,
    subagent_timeout_secs: u64,
    agent_generation: u64,
}

impl DaemonToolPort {
    pub fn new(
        client: DaemonClient,
        event_tx: mpsc::UnboundedSender<AppEvent>,
        hook_manager: Arc<HookManager>,
        session_id: String,
        subagent_timeout_secs: u64,
        agent_generation: u64,
    ) -> Self {
        Self {
            client,
            event_tx,
            hook_manager,
            session_id,
            subagent_timeout_secs,
            agent_generation,
        }
    }

    fn format_resp(resp: &crate::tui::client::ExecuteToolResponse) -> String {
        format!(
            r#"{{"success":{},"output_type":{},"content":{},"error":{},"metadata":{}}}"#,
            resp.success,
            serde_json::to_string(&resp.output_type).unwrap_or_default(),
            serde_json::to_string(&resp.content).unwrap_or_default(),
            serde_json::to_string(&resp.error).unwrap_or_default(),
            serde_json::to_string(&resp.metadata).unwrap_or_default(),
        )
    }

    fn spawn_progress_poller(&self) -> tokio::task::JoinHandle<()> {
        let tx = self.event_tx.clone();
        let client = self.client.clone();
        let session_id = self.session_id.clone();
        let max_duration = Duration::from_secs(self.subagent_timeout_secs);
        let generation = self.agent_generation;
        tokio::spawn(async move {
            let start = tokio::time::Instant::now();
            // Track which subagent permission requests we've already prompted for.
            let mut prompted: std::collections::HashSet<String> = std::collections::HashSet::new();
            loop {
                if start.elapsed() > max_duration {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(500)).await;

                // Drain subagent policy-Ask approvals into the existing PermissionRequired UI.
                if let Ok(pending) = client.list_pending_permissions().await {
                    for item in pending {
                        if !prompted.insert(item.request_id.clone()) {
                            continue;
                        }
                        let (resp_tx, resp_rx) = tokio::sync::oneshot::channel();
                        let reason = if item.human_summary.is_empty() {
                            format!(
                                "Subagent `{}` needs permission for `{}`: {}",
                                item.from, item.tool, item.policy_reason
                            )
                        } else {
                            format!("Subagent `{}`: {}", item.from, item.human_summary)
                        };
                        let _ = tx.send(AppEvent::PermissionRequired {
                            tool_name: item.tool.clone(),
                            reason,
                            rule: item.session_rule.clone(),
                            responder: crate::tui::app::PermissionResponder(Some(resp_tx)),
                        });
                        let client2 = client.clone();
                        let request_id = item.request_id.clone();
                        let session_rule = item.session_rule.clone();
                        tokio::spawn(async move {
                            let decision = resp_rx
                                .await
                                .unwrap_or(crate::tui::app::PermissionResponse::Deny);
                            let (approved, always) = match decision {
                                crate::tui::app::PermissionResponse::AllowOnce => (true, false),
                                crate::tui::app::PermissionResponse::AlwaysAllow => (true, true),
                                crate::tui::app::PermissionResponse::Deny => (false, false),
                            };
                            let _ = client2
                                .resolve_subagent_permission(
                                    &request_id,
                                    approved,
                                    always,
                                    Some(session_rule.as_str()),
                                )
                                .await;
                        });
                    }
                }

                match client.get_root_agent_view(&session_id).await {
                    Ok(view) => {
                        let all_terminal = !view.children.is_empty()
                            && view.children.iter().all(|c| c.status.is_terminal());
                        let _ = tx.send(AppEvent::AgentLocalView {
                            view: Box::new(view),
                            generation,
                        });
                        if all_terminal {
                            break;
                        }
                    }
                    Err(error) => {
                        tracing::warn!(
                            session_id = %session_id,
                            error = %error,
                            "Failed to poll scoped subagent view; retrying"
                        );
                    }
                }
            }
        })
    }
}

#[async_trait]
impl ToolPort for DaemonToolPort {
    async fn execute(&self, req: ToolRequest) -> ToolResponse {
        // Guardian: critical-risk shell commands never reach the daemon.
        if req.name == "execute_command" || req.name == "exec_command" {
            if let Some(cmd) = req.arguments.get("command").and_then(|v| v.as_str()) {
                let risk = classify_risk(cmd);
                if risk >= crate::runtime::guardian::RiskLevel::Critical {
                    let msg = format!("GUARDIAN BLOCK: critical-risk command rejected. {}", cmd);
                    tracing::warn!("{}", msg);
                    return ToolResponse {
                        content: format!(r#"{{"success":false,"error":"{}"}}"#, msg),
                        success: false,
                    };
                }
            }
        }

        let is_long = req.name == "task" || req.name == "delegate";
        let poll_handle = if is_long && !self.event_tx.is_closed() {
            Some(self.spawn_progress_poller())
        } else {
            None
        };

        if req.parallel {
            // No interactive permission in parallel batches.
            let content = match self
                .client
                .execute_tool(
                    &req.name,
                    req.arguments.clone(),
                    &req.session_id,
                    req.turn_id.as_deref(),
                )
                .await
            {
                Ok(resp) => {
                    if let Some(perm) = resp.permission_required {
                        format!(
                            r#"{{"success":false,"error":"PERMISSION REQUIRED: {} (cannot prompt in parallel mode)"}}"#,
                            perm.reason
                        )
                    } else {
                        Self::format_resp(&resp)
                    }
                }
                Err(e) => format!(r#"{{"success":false,"error":"{}"}}"#, e),
            };
            // Detach poller (do not abort) — see historical comments on task fire-and-forget.
            drop(poll_handle);
            return ToolResponse {
                success: content.contains("\"success\":true"),
                content,
            };
        }

        // Sequential path with permission UI.
        let result = match self
            .client
            .execute_tool(
                &req.name,
                req.arguments.clone(),
                &req.session_id,
                req.turn_id.as_deref(),
            )
            .await
        {
            Ok(resp) => resp,
            Err(e) => {
                drop(poll_handle);
                tracing::warn!("Tool execution failed for '{}': {}", req.name, e);
                return ToolResponse {
                    content: format!(r#"{{"success":false,"error":"{}"}}"#, e),
                    success: false,
                };
            }
        };

        if let Some(perm) = result.permission_required {
            tracing::info!(
                "🔐 Permission required for '{}': {} (rule: {})",
                req.name,
                perm.reason,
                perm.session_rule
            );

            {
                let hm = self.hook_manager.clone();
                let hook_name = req.name.clone();
                let hook_args = req.arguments.clone();
                let hook_sid = req.session_id.clone();
                tokio::spawn(async move {
                    let cwd = std::env::current_dir().unwrap_or_default();
                    let ctx = crate::runtime::hooks::HookContext {
                        event: "PermissionRequest".to_string(),
                        tool_name: Some(hook_name),
                        tool_input: Some(hook_args),
                        tool_result: None,
                        session_id: Some(hook_sid),
                        working_directory: cwd.to_string_lossy().to_string(),
                        timestamp: chrono::Utc::now().to_rfc3339(),
                        comet_phase: None,
                        workflow_state: None,
                        variables: Default::default(),
                    };
                    hm.fire(
                        &crate::runtime::hooks::HookEvent::PermissionRequest,
                        &ctx,
                        None,
                        None,
                    )
                    .await;
                });
            }

            let (tx, rx) = tokio::sync::oneshot::channel();
            // Prefer daemon-provided tool_name; fall back to the request name
            // for older daemon payloads that omit the field.
            let tool_name = if perm.tool_name.is_empty() {
                req.name.clone()
            } else {
                perm.tool_name.clone()
            };
            let _ = self.event_tx.send(AppEvent::PermissionRequired {
                tool_name,
                reason: perm.reason.clone(),
                rule: perm.session_rule.clone(),
                responder: crate::tui::app::PermissionResponder(Some(tx)),
            });

            let content = match rx.await {
                Ok(crate::tui::app::PermissionResponse::AllowOnce) => {
                    if self.client.approve_tool(&perm.session_rule).await.is_err() {
                        drop(poll_handle);
                        return ToolResponse {
                            content: r#"{"success":false,"error":"Failed to approve permission"}"#
                                .to_string(),
                            success: false,
                        };
                    }
                    let result = self
                        .client
                        .execute_tool(
                            &req.name,
                            req.arguments.clone(),
                            &req.session_id,
                            req.turn_id.as_deref(),
                        )
                        .await;
                    let _ = self.client.unapprove_tool(&perm.session_rule).await;
                    match result {
                        Ok(resp) => Self::format_resp(&resp),
                        Err(e) => format!(r#"{{"success":false,"error":"{}"}}"#, e),
                    }
                }
                Ok(crate::tui::app::PermissionResponse::AlwaysAllow) => {
                    if self.client.approve_tool(&perm.session_rule).await.is_err() {
                        drop(poll_handle);
                        return ToolResponse {
                            content: r#"{"success":false,"error":"Failed to approve permission"}"#
                                .to_string(),
                            success: false,
                        };
                    }
                    match self
                        .client
                        .execute_tool(
                            &req.name,
                            req.arguments.clone(),
                            &req.session_id,
                            req.turn_id.as_deref(),
                        )
                        .await
                    {
                        Ok(resp) => Self::format_resp(&resp),
                        Err(e) => format!(r#"{{"success":false,"error":"{}"}}"#, e),
                    }
                }
                Ok(crate::tui::app::PermissionResponse::Deny) | Err(_) => {
                    format!(
                        r#"{{"success":false,"error":"PERMISSION DENIED: {}"}}"#,
                        perm.reason
                    )
                }
            };
            drop(poll_handle);
            return ToolResponse {
                success: content.contains("\"success\":true"),
                content,
            };
        }

        drop(poll_handle);
        let content = Self::format_resp(&result);
        ToolResponse {
            success: result.success,
            content,
        }
    }

    fn definitions(&self) -> Vec<ToolDefinition> {
        // Daemon injects tools server-side for chat streams.
        Vec::new()
    }
}

// ── Interaction (ask_user_question) ─────────────────────────────────────────

pub struct TuiInteractionPort {
    event_tx: mpsc::UnboundedSender<AppEvent>,
}

impl TuiInteractionPort {
    pub fn new(event_tx: mpsc::UnboundedSender<AppEvent>) -> Self {
        Self { event_tx }
    }
}

#[async_trait]
impl InteractionPort for TuiInteractionPort {
    async fn ask_user_question(&self, args: &serde_json::Value) -> String {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let question = args["question"]
            .as_str()
            .unwrap_or("Choose an option:")
            .to_string();
        let options: Vec<QuestionOption> = args["options"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|o| {
                        let label = o["label"].as_str()?;
                        let description = o["description"].as_str().unwrap_or("").to_string();
                        Some(QuestionOption {
                            label: label.to_string(),
                            description,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        // Schema declares `multiSelect` (camelCase), which is what the LLM
        // produces. Accept `multi_select` (snake_case) as a robustness fallback
        // for models that emit snake_case.
        let multi_select = args["multiSelect"]
            .as_bool()
            .or_else(|| args["multi_select"].as_bool())
            .unwrap_or(false);

        let _ = self.event_tx.send(AppEvent::QuestionAsked {
            question,
            options,
            multi_select,
            responder: crate::tui::app::QuestionResponder(Some(tx)),
        });

        match rx.await {
            Ok(answers) => {
                let answers_json: Vec<serde_json::Value> = answers
                    .iter()
                    .map(|a| serde_json::json!({"label": a, "value": a, "custom": false}))
                    .collect();
                serde_json::json!({
                    "success": true,
                    "answers": answers_json
                })
                .to_string()
            }
            Err(_) => serde_json::json!({
                "success": false,
                "error": "User cancelled the question"
            })
            .to_string(),
        }
    }
}

// ── Compactor (transcript + summary stream + memory extract) ────────────────

pub struct TuiCompactor {
    client: DaemonClient,
    event_tx: mpsc::UnboundedSender<AppEvent>,
    assembled_system_messages: Vec<ChatMessage>,
    memory_manager: Arc<MemoryManager>,
    /// Written on success so the AgentLoop can surface summary size.
    compacted_summary: Arc<tokio::sync::Mutex<String>>,
}

impl TuiCompactor {
    pub fn new(
        client: DaemonClient,
        event_tx: mpsc::UnboundedSender<AppEvent>,
        assembled_system_messages: Vec<ChatMessage>,
        memory_manager: Arc<MemoryManager>,
        compacted_summary: Arc<tokio::sync::Mutex<String>>,
    ) -> Self {
        Self {
            client,
            event_tx,
            assembled_system_messages,
            memory_manager,
            compacted_summary,
        }
    }
}

#[async_trait]
impl Compactor for TuiCompactor {
    async fn compact(&self, history: &dyn HistoryStore) -> bool {
        let history_snapshot = history.get().await;
        archive_transcript(&history_snapshot).await;

        let (tail, transcript_text) =
            crate::agent::runtime::compactor::prepare_compaction_transcript(&history_snapshot);

        let summary_messages = vec![
            ChatMessage::system(COMPACTION_SYSTEM_PROMPT),
            ChatMessage::user(format!(
                "Process this conversation history:\n\n{}",
                transcript_text
            )),
        ];

        // plan_mode=true makes the daemon omit tools so the model cannot
        // answer with a tool_call (empty content → silent compaction failure).
        let response = match self
            .client
            .chat_stream_with_plan(summary_messages, None, Some(true))
            .await
        {
            Ok(r) => r,
            Err(e) => {
                let err = e.to_string();
                tracing::warn!(error = %err, "compaction summary request failed");
                if crate::agent::runtime::compactor::is_payload_too_large_error(&err)
                    && crate::agent::runtime::compactor::fallback_micro_compact(history).await
                {
                    let _ = self.event_tx.send(AppEvent::StreamError(
                        "Compaction summary hit a size limit; applied micro-compact fallback."
                            .to_string(),
                    ));
                    return true;
                }
                let _ = self.event_tx.send(AppEvent::StreamError(
                    "Compaction failed; continuing with full history.".to_string(),
                ));
                return false;
            }
        };

        let mut processor = StreamProcessor::new();
        let mut stream = response.bytes_stream();
        let mut stream_error: Option<String> = None;
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(bytes) => {
                    for ev in processor.feed_bytes(&bytes) {
                        if let StreamEvent::StreamError(msg) = ev {
                            stream_error = Some(msg);
                        }
                    }
                }
                Err(e) => {
                    stream_error = Some(format!("summary stream read error: {e}"));
                    break;
                }
            }
        }
        for ev in processor.flush() {
            if let StreamEvent::StreamError(msg) = ev {
                stream_error = Some(msg);
            }
        }
        if let Some(reason) = stream_error {
            tracing::warn!(reason = %reason, "compaction summary stream errored");
            if crate::agent::runtime::compactor::is_payload_too_large_error(&reason)
                && crate::agent::runtime::compactor::fallback_micro_compact(history).await
            {
                let _ = self.event_tx.send(AppEvent::StreamError(
                    "Compaction summary stream hit a size limit; applied micro-compact fallback."
                        .to_string(),
                ));
                return true;
            }
            let _ = self.event_tx.send(AppEvent::StreamError(
                "Compaction failed; continuing with full history.".to_string(),
            ));
            return false;
        }

        let result = processor.finish();
        let full_text = if !result.content.is_empty() {
            result.content
        } else {
            result.reasoning_content
        };

        let (summary, extracted_memories) = parse_compaction_response(&full_text);

        if summary.trim().is_empty() {
            tracing::warn!("compaction produced an empty summary; leaving history intact");
            // Empty summary after a large request: still try micro-compact so we
            // make progress instead of permanently disabling compaction.
            if crate::agent::runtime::compactor::fallback_micro_compact(history).await {
                let _ = self.event_tx.send(AppEvent::StreamError(
                    "Compaction produced an empty summary; applied micro-compact fallback."
                        .to_string(),
                ));
                return true;
            }
            let _ = self.event_tx.send(AppEvent::StreamError(
                "Compaction produced an empty summary; continuing with full history.".to_string(),
            ));
            return false;
        }

        let filtered = crate::agent::runtime::compactor::filter_extracted_memories(
            extracted_memories,
            self.memory_manager.write_importance_threshold(),
            self.memory_manager.max_extract_per_compaction(),
        );
        for (memory, scope) in &filtered {
            if let Err(e) = self.memory_manager.add_memory(memory.clone(), *scope).await {
                tracing::warn!(error = %e, memory_id = %memory.id, "failed to persist extracted memory");
            }
        }
        if !filtered.is_empty() {
            tracing::info!(count = filtered.len(), "extracted memories from compaction");
        }

        {
            *self.compacted_summary.lock().await = summary.clone();
        }
        let new_history =
            assemble_post_compaction_history(&self.assembled_system_messages, &summary, &tail);
        history.replace(new_history).await;

        let summary_chars = summary.chars().count();
        let _ = self
            .event_tx
            .send(AppEvent::ContextCompacted { summary_chars });
        true
    }
}

// ── Task progress (daemon) ───────────────────────────────────────────────────

/// [`TaskProgressPort`] backed by the daemon `/tasks/progress` endpoint.
pub struct DaemonTaskProgress {
    client: DaemonClient,
}

impl DaemonTaskProgress {
    pub fn new(client: DaemonClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl crate::agent::runtime::TaskProgressPort for DaemonTaskProgress {
    async fn blocked_and_ready(&self) -> (usize, usize) {
        // Opportunistic: a fetch failure just skips the nudge this round.
        match self.client.task_progress().await {
            Ok(resp) => (resp.blocked, resp.ready),
            Err(e) => {
                tracing::debug!(error = %e, "task_progress fetch failed; skipping nudge");
                (0, 0)
            }
        }
    }
}

// ── Planner ─────────────────────────────────────────────────────────────────

pub struct ApiPlannerPort {
    client: ApiClient,
}

impl ApiPlannerPort {
    pub fn new(client: ApiClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl PlannerPort for ApiPlannerPort {
    async fn plan(&self, messages: &[ChatMessage]) -> Result<String, String> {
        let response = self
            .client
            .chat(messages.to_vec(), None)
            .await
            .map_err(|e| format!("Planner model call failed: {}", e))?;
        Ok(response
            .choices
            .first()
            .map(|c| c.message.content.clone().unwrap_or_default())
            .unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::app::AppEvent;

    #[tokio::test]
    async fn ask_user_question_reads_multiselect_camelcase() {
        // The tool schema declares `multiSelect` (camelCase); the adapter must
        // read that key (not `multi_select`) so multi-select actually activates.
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<AppEvent>();
        let port = TuiInteractionPort::new(tx);

        let args = serde_json::json!({
            "question": "Which frameworks?",
            "options": [
                {"label": "React", "description": "d"},
                {"label": "Vue", "description": "d"},
                {"label": "Svelte", "description": "d"}
            ],
            "multiSelect": true
        });

        // Drive the port in a spawned task so we can receive the event and
        // respond via the oneshot while the call is in flight.
        let handle = tokio::spawn(async move { port.ask_user_question(&args).await });

        // The panel should receive a multi-select question.
        let event = rx.recv().await.expect("event sent");
        match event {
            AppEvent::QuestionAsked {
                multi_select,
                responder,
                ..
            } => {
                assert!(multi_select, "multiSelect must propagate as true");

                // Simulate the user checking two options and submitting.
                let sender = responder.0.expect("responder sender present");
                sender
                    .send(vec!["React".to_string(), "Svelte".to_string()])
                    .expect("send answers");
            }
            other => panic!("expected QuestionAsked, got {other:?}"),
        }

        let result = handle.await.expect("task join");
        let parsed: serde_json::Value = serde_json::from_str(&result).expect("json output");
        assert_eq!(parsed["success"], true);
        let labels: Vec<&str> = parsed["answers"]
            .as_array()
            .expect("answers array")
            .iter()
            .map(|a| a["label"].as_str().expect("label"))
            .collect();
        assert_eq!(labels, vec!["React", "Svelte"]);
    }

    #[tokio::test]
    async fn ask_user_question_accepts_snake_case_fallback() {
        // Some models emit snake_case; the adapter falls back to `multi_select`.
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<AppEvent>();
        let port = TuiInteractionPort::new(tx);

        let args = serde_json::json!({
            "question": "q",
            "options": [{"label": "a", "description": "d"}],
            "multi_select": true
        });

        let handle = tokio::spawn(async move { port.ask_user_question(&args).await });

        let event = rx.recv().await.expect("event sent");
        match event {
            AppEvent::QuestionAsked {
                multi_select,
                responder,
                ..
            } => {
                assert!(
                    multi_select,
                    "snake_case fallback must also activate multi-select"
                );
                let _ = responder.0.expect("sender").send(vec!["a".to_string()]);
            }
            other => panic!("expected QuestionAsked, got {other:?}"),
        }
        let result = handle.await.expect("task join");
        let parsed: serde_json::Value = serde_json::from_str(&result).expect("json output");
        assert_eq!(parsed["success"], true);
    }
}
