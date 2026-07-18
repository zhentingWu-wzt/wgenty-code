//! Mock-based tests for the shared `run_agent_loop`.
//!
//! These exercise the loop's control flow without a real LLM: a scripted
//! [`LlmPort`] returns canned `ChatCompletion`s, a [`MockToolPort`] returns
//! canned tool results, and a [`VecSink`] records emitted events.

use super::ports::{
    ChatCompletion, Compactor, EventSink, HistoryStore, LlmPort, TaskProgressPort, ToolPort,
    ToolRequest, ToolResponse,
};
use super::{
    estimate_prompt_tokens, run_agent_loop, LoopHooks, LoopTurnState, RunLoopArgs, RuntimeConfig,
    RuntimeError, RuntimeEvent, StreamStyle,
};
use crate::agent::runtime::MutexHistoryStore;
use crate::api::token_counter::TokenCounter;
use crate::api::{ChatMessage, ToolCall, ToolCallFunction, ToolDefinition, Usage};
use crate::utils::stuck_detector::StuckDetector;
use async_trait::async_trait;
use bytes::Bytes;
use futures::stream::BoxStream;
use std::sync::{Arc, Mutex};
use tokio::sync::Mutex as TokioMutex;

// ── Mocks ───────────────────────────────────────────────────────────────────

struct ScriptedLlm {
    /// Pre-seeded responses, popped in order per `chat_completion` call.
    responses: Arc<TokioMutex<Vec<ChatCompletion>>>,
    calls: Arc<Mutex<usize>>,
}

impl ScriptedLlm {
    fn new(responses: Vec<ChatCompletion>) -> Self {
        Self {
            responses: Arc::new(TokioMutex::new(responses)),
            calls: Arc::new(Mutex::new(0)),
        }
    }

    fn call_count(&self) -> usize {
        *self.calls.lock().unwrap()
    }
}

#[async_trait]
impl LlmPort for ScriptedLlm {
    async fn open_chat_stream(
        &self,
        _messages: Vec<ChatMessage>,
        _tools: Option<Vec<ToolDefinition>>,
        _max_tokens: Option<usize>,
        _plan_mode: Option<bool>,
    ) -> Result<BoxStream<'static, Result<Bytes, RuntimeError>>, RuntimeError> {
        Err(RuntimeError::Stream(
            "ScriptedLlm is non-stream only".into(),
        ))
    }

    async fn chat_completion(
        &self,
        _messages: Vec<ChatMessage>,
        _tools: Option<Vec<ToolDefinition>>,
    ) -> Result<ChatCompletion, RuntimeError> {
        *self.calls.lock().unwrap() += 1;
        let mut store = self.responses.lock().await;
        if store.is_empty() {
            return Err(RuntimeError::EmptyResponse);
        }
        Ok(store.remove(0))
    }
}

struct MockToolPort {
    /// tool_name -> canned result content.
    results: std::collections::HashMap<String, String>,
    calls: Arc<Mutex<Vec<(String, serde_json::Value)>>>,
}

impl MockToolPort {
    fn new() -> Self {
        Self {
            results: std::collections::HashMap::new(),
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn with_result(mut self, name: &str, content: &str) -> Self {
        self.results.insert(name.to_string(), content.to_string());
        self
    }

    fn recorded(&self) -> Vec<(String, String)> {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .map(|(n, a)| (n.clone(), a.to_string()))
            .collect()
    }
}

#[async_trait]
impl ToolPort for MockToolPort {
    async fn execute(&self, req: ToolRequest) -> ToolResponse {
        self.calls
            .lock()
            .unwrap()
            .push((req.name.clone(), req.arguments.clone()));
        let content = self.results.get(&req.name).cloned().unwrap_or_else(|| {
            format!(r#"{{"success":false,"error":"no mock for {0}"}}"#, req.name)
        });
        let success = content.contains("\"success\":true");
        ToolResponse { content, success }
    }

    fn definitions(&self) -> Vec<ToolDefinition> {
        Vec::new()
    }
}

struct VecSink {
    events: Arc<Mutex<Vec<RuntimeEvent>>>,
}

impl VecSink {
    fn new() -> Self {
        Self {
            events: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn snapshot(&self) -> Vec<String> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .map(|e| match e {
                RuntimeEvent::ContentDelta(t) => format!("delta:{t}"),
                RuntimeEvent::ToolStart { name, .. } => format!("tool_start:{name}"),
                RuntimeEvent::ToolResult { name, .. } => format!("tool_result:{name}"),
                RuntimeEvent::StreamError(m) => format!("error:{m}"),
                RuntimeEvent::StreamDone { finish_reason } => format!("done:{finish_reason}"),
                RuntimeEvent::SaveSession => "save".to_string(),
                _ => "?".to_string(),
            })
            .collect()
    }
}

impl EventSink for VecSink {
    fn emit(&self, event: RuntimeEvent) {
        self.events.lock().unwrap().push(event);
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn text_response(text: &str) -> ChatCompletion {
    ChatCompletion {
        message: ChatMessage::assistant(text),
        finish_reason: "stop".to_string(),
        usage: Some(Usage {
            prompt_tokens: 10,
            completion_tokens: 5,
            total_tokens: 15,
        }),
    }
}

fn tool_call_response(id: &str, name: &str, args: &str) -> ChatCompletion {
    ChatCompletion {
        message: ChatMessage {
            role: "assistant".to_string(),
            content: None,
            reasoning_content: None,
            tool_calls: Some(vec![ToolCall {
                id: id.to_string(),
                r#type: "function".to_string(),
                function: ToolCallFunction {
                    name: name.to_string(),
                    arguments: args.to_string(),
                },
            }]),
            tool_call_id: None,
        },
        finish_reason: "tool_calls".to_string(),
        usage: None,
    }
}

fn default_config() -> RuntimeConfig {
    RuntimeConfig {
        max_rounds: 20,
        plan_mode: false,
        subagent_timeout_secs: 60,
        context_window: 200_000,
        max_tokens: 4096,
        session_id: "test".to_string(),
        turn_id: None,
        agent_generation: 0,
        stream_max_retries: 0,
    }
}

async fn run(
    llm: &ScriptedLlm,
    tools: &MockToolPort,
    events: &VecSink,
    history: &MutexHistoryStore,
    config: &RuntimeConfig,
    state: &mut LoopTurnState,
) -> Result<String, RuntimeError> {
    run_agent_loop(RunLoopArgs {
        llm,
        tools,
        events,
        history,
        config,
        state,
        stream_style: StreamStyle::subagent(),
        hooks: LoopHooks::default(),
        system_messages: &[],
    })
    .await
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[tokio::test]
async fn final_text_turn_emits_content_and_save() {
    let llm = ScriptedLlm::new(vec![text_response("done")]);
    let tools = MockToolPort::new();
    let events = VecSink::new();
    let history = MutexHistoryStore::new(Arc::new(TokioMutex::new(vec![ChatMessage::user("hi")])));

    let mut state = LoopTurnState::default();
    let out = run(
        &llm,
        &tools,
        &events,
        &history,
        &default_config(),
        &mut state,
    )
    .await
    .unwrap();

    assert_eq!(out, "done");
    assert_eq!(llm.call_count(), 1);
    let snap = events.snapshot();
    assert!(snap.iter().any(|s| s == "delta:done"));
    assert!(snap.iter().any(|s| s == "save"));
    assert_eq!(tools.recorded().len(), 0);
}

#[tokio::test]
async fn multi_round_tool_use_then_finalize() {
    let llm = ScriptedLlm::new(vec![
        tool_call_response("c1", "file_read", r#"{"path":"a"}"#),
        text_response("summary"),
    ]);
    let tools =
        MockToolPort::new().with_result("file_read", r#"{"success":true,"content":"hello"}"#);
    let events = VecSink::new();
    let history =
        MutexHistoryStore::new(Arc::new(TokioMutex::new(vec![ChatMessage::user("read a")])));

    let mut state = LoopTurnState::default();
    let out = run(
        &llm,
        &tools,
        &events,
        &history,
        &default_config(),
        &mut state,
    )
    .await
    .unwrap();

    assert_eq!(out, "summary");
    assert_eq!(llm.call_count(), 2);
    let recorded = tools.recorded();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].0, "file_read");
    // Final history contains tool result message.
    let hist = history.get().await;
    assert!(hist.iter().any(|m| m.role == "tool"));
}

#[tokio::test]
async fn irrecoverable_parse_errors_abort() {
    // Three tool calls with garbage JSON -> abort.
    let bad = r#"{not json"#;
    let llm = ScriptedLlm::new(vec![
        tool_call_response("c1", "grep", bad),
        tool_call_response("c2", "grep", bad),
        tool_call_response("c3", "grep", bad),
    ]);
    let tools = MockToolPort::new();
    let events = VecSink::new();
    let history = MutexHistoryStore::new(Arc::new(TokioMutex::new(vec![ChatMessage::user("x")])));

    let mut state = LoopTurnState::default();
    let result = run(
        &llm,
        &tools,
        &events,
        &history,
        &default_config(),
        &mut state,
    )
    .await;

    assert!(result.is_err());
    let snap = events.snapshot();
    assert!(snap.iter().any(|s| s.starts_with("error:")));
    // Tool never executed with garbage args.
    assert_eq!(tools.recorded().len(), 0);
}

#[tokio::test]
async fn max_rounds_exceeded_aborts() {
    // Always return a tool call so it never finalizes.
    let mut responses = Vec::new();
    for i in 0..30 {
        responses.push(tool_call_response(
            &format!("c{i}"),
            "file_read",
            r#"{"path":"a"}"#,
        ));
    }
    let llm = ScriptedLlm::new(responses);
    let tools = MockToolPort::new().with_result("file_read", r#"{"success":true,"content":"x"}"#);
    let events = VecSink::new();
    let history =
        MutexHistoryStore::new(Arc::new(TokioMutex::new(vec![ChatMessage::user("loop")])));
    let mut config = default_config();
    config.max_rounds = 3;

    let mut state = LoopTurnState::default();
    let result = run(&llm, &tools, &events, &history, &config, &mut state).await;
    assert!(matches!(
        result,
        Err(RuntimeError::MaxRoundsExceeded { .. })
    ));
}

#[tokio::test]
async fn stuck_detector_aborts_on_repeat() {
    // Repeated identical tool call many times triggers StuckStatus::Abort.
    let mut responses = Vec::new();
    for i in 0..20 {
        responses.push(tool_call_response(
            &format!("c{i}"),
            "file_read",
            r#"{"path":"a"}"#,
        ));
    }
    let llm = ScriptedLlm::new(responses);
    let tools = MockToolPort::new().with_result("file_read", r#"{"success":true,"content":"x"}"#);
    let events = VecSink::new();
    let history =
        MutexHistoryStore::new(Arc::new(TokioMutex::new(vec![ChatMessage::user("stuck")])));
    let mut stuck = StuckDetector::new();

    let mut state = LoopTurnState::default();
    let result = run_agent_loop(RunLoopArgs {
        llm: &llm,
        tools: &tools,
        events: &events,
        history: &history,
        config: &default_config(),
        state: &mut state,
        stream_style: StreamStyle::subagent(),
        hooks: LoopHooks {
            stuck_detector: Some(&mut stuck),
            ..LoopHooks::default()
        },
        system_messages: &[],
    })
    .await;

    assert!(result.is_err());
}

#[tokio::test]
async fn recoverable_parse_still_executes_tool() {
    // Truncated but recoverable JSON with a real key -> tool runs, result pushed.
    let llm = ScriptedLlm::new(vec![
        tool_call_response("c1", "file_read", r#"{"path":"a""#), // missing closing brace
        text_response("ok"),
    ]);
    let tools =
        MockToolPort::new().with_result("file_read", r#"{"success":true,"content":"data"}"#);
    let events = VecSink::new();
    let history =
        MutexHistoryStore::new(Arc::new(TokioMutex::new(vec![ChatMessage::user("read")])));

    let mut state = LoopTurnState::default();
    let out = run(
        &llm,
        &tools,
        &events,
        &history,
        &default_config(),
        &mut state,
    )
    .await
    .unwrap();
    assert_eq!(out, "ok");
    assert_eq!(tools.recorded().len(), 1);
}

// ── Task progress nudge ─────────────────────────────────────────────────────

struct FixedTaskProgress {
    blocked: usize,
    ready: usize,
}

#[async_trait::async_trait]
impl TaskProgressPort for FixedTaskProgress {
    async fn blocked_and_ready(&self) -> (usize, usize) {
        (self.blocked, self.ready)
    }
}

#[tokio::test]
async fn ready_task_nudge_injected_after_idle_rounds() {
    // Model keeps calling a non-task tool; after 3 idle rounds with ready>0,
    // a <reminder> about ready tasks is appended to the last tool result.
    let mut responses = Vec::new();
    for i in 0..6 {
        responses.push(tool_call_response(
            &format!("c{i}"),
            "file_read",
            r#"{"path":"a"}"#,
        ));
    }
    responses.push(text_response("done"));
    let llm = ScriptedLlm::new(responses);
    let tools = MockToolPort::new().with_result("file_read", r#"{"success":true,"content":"x"}"#);
    let events = VecSink::new();
    let history =
        MutexHistoryStore::new(Arc::new(TokioMutex::new(vec![ChatMessage::user("work")])));

    let progress = FixedTaskProgress {
        blocked: 1,
        ready: 2,
    };

    let mut state = LoopTurnState::default();
    let _ = run_agent_loop(RunLoopArgs {
        llm: &llm,
        tools: &tools,
        events: &events,
        history: &history,
        config: &default_config(),
        state: &mut state,
        stream_style: StreamStyle::subagent(),
        hooks: LoopHooks {
            task_progress: Some(&progress),
            ..LoopHooks::default()
        },
        system_messages: &[],
    })
    .await
    .unwrap();

    let hist = history.get().await;
    let nudged = hist
        .iter()
        .filter(|m| m.role == "tool")
        .filter_map(|m| m.content.as_deref())
        .any(|c| c.contains("ready") && c.contains("task_management"));
    assert!(nudged, "expected a ready-task reminder in tool results");
}

#[tokio::test]
async fn no_nudge_when_nothing_ready() {
    let mut responses = Vec::new();
    for i in 0..5 {
        responses.push(tool_call_response(
            &format!("c{i}"),
            "file_read",
            r#"{"path":"a"}"#,
        ));
    }
    responses.push(text_response("done"));
    let llm = ScriptedLlm::new(responses);
    let tools = MockToolPort::new().with_result("file_read", r#"{"success":true,"content":"x"}"#);
    let events = VecSink::new();
    let history =
        MutexHistoryStore::new(Arc::new(TokioMutex::new(vec![ChatMessage::user("work")])));

    let progress = FixedTaskProgress {
        blocked: 3,
        ready: 0,
    };

    let mut state = LoopTurnState::default();
    let _ = run_agent_loop(RunLoopArgs {
        llm: &llm,
        tools: &tools,
        events: &events,
        history: &history,
        config: &default_config(),
        state: &mut state,
        stream_style: StreamStyle::subagent(),
        hooks: LoopHooks {
            task_progress: Some(&progress),
            ..LoopHooks::default()
        },
        system_messages: &[],
    })
    .await
    .unwrap();

    let hist = history.get().await;
    let nudged = hist
        .iter()
        .filter(|m| m.role == "tool")
        .filter_map(|m| m.content.as_deref())
        .any(|c| c.contains("ready") && c.contains("task_management"));
    assert!(!nudged, "no ready-task reminder when ready==0");
}

/// Compactor that returns a short summary without mutating history.
struct ShrinkCompactor;

#[async_trait]
impl Compactor for ShrinkCompactor {
    async fn compact(&self, _history: &dyn HistoryStore) -> Option<String> {
        Some("short".to_string())
    }
}

#[tokio::test]
async fn successful_compaction_updates_last_prompt_tokens() {
    // No usage in the post-compact LLM response so last_prompt_tokens keeps the
    // estimate written immediately after the compaction summary is applied.
    let llm = ScriptedLlm::new(vec![ChatCompletion {
        message: ChatMessage::assistant("ok"),
        finish_reason: "stop".to_string(),
        usage: None,
    }]);
    let tools = MockToolPort::new();
    let events = VecSink::new();
    let bulky = "x".repeat(400);
    let system_messages = vec![ChatMessage::system("sys")];
    let history =
        MutexHistoryStore::new(Arc::new(TokioMutex::new(vec![ChatMessage::user(&bulky)])));
    let counter = TokenCounter::new();
    // Stale pre-compact estimate the UI would still show without the fix.
    counter.set_prompt_tokens(50_000);

    let mut state = LoopTurnState {
        compact_requested: true,
        ..LoopTurnState::default()
    };
    let compactor = ShrinkCompactor;
    let _ = run_agent_loop(RunLoopArgs {
        llm: &llm,
        tools: &tools,
        events: &events,
        history: &history,
        config: &default_config(),
        state: &mut state,
        stream_style: StreamStyle::subagent(),
        hooks: LoopHooks {
            compactor: Some(&compactor),
            token_counter: Some(&counter),
            ..LoopHooks::default()
        },
        system_messages: &system_messages,
    })
    .await
    .unwrap();

    // Estimate is taken right after the compaction summary is applied (before
    // the next assistant reply). The post-compaction API view is
    // system_messages + summary + synthetic user + (empty tail).
    let expected = estimate_prompt_tokens(&super::compaction::assemble_post_compaction_history(
        &system_messages,
        "short",
        &[],
    ));
    assert_eq!(
        counter.last_prompt_tokens(),
        expected,
        "context bar must refresh from post-compact view estimate"
    );
    assert!(counter.last_prompt_tokens() < 50_000);
    // History is preserved verbatim (not mutated by compaction).
    let hist = history.get().await;
    assert!(
        hist.iter()
            .any(|m| m.content.as_deref() == Some(bulky.as_str())),
        "full history should be preserved after compaction"
    );
    assert!(
        !hist.iter().any(|m| m.content.as_deref() == Some("short")),
        "compaction summary should NOT be stored in history"
    );
}
