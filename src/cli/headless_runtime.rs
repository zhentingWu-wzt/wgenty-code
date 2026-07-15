//! Headless agent runtime used by `wgenty-code query`.
//!
//! Builds in-process ports (ApiClient + ToolRegistry) and runs the shared
//! multi-round loop so CLI gets the same tool / compaction policy as TUI
//! (micro-compact always; auto-summary via [`ApiCompactor`]).

use crate::agent::runtime::{
    run_agent_loop, ApiCompactor, ApiLlmPort, EventSink, LoopHooks, LoopTurnState,
    MutexHistoryStore, RunLoopArgs, RuntimeConfig, RuntimeEvent, StreamStyle, ToolPort,
    ToolRequest, ToolResponse,
};
use crate::agent::{AgentExecutionContext, SessionId, ToolContext, ToolInvocationId};
use crate::api::{ApiClient, ChatMessage, ToolDefinition};
use crate::config::Settings;
use crate::context::MemoryManager;
use crate::prompts::{self, PromptContext};
use crate::tools::ToolRegistry;
use async_trait::async_trait;
use std::io::{self, Write};
use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid;

/// stdout/stderr event sink for headless runs.
pub struct CliEventSink {
    verbose: bool,
}

impl CliEventSink {
    pub fn new(verbose: bool) -> Self {
        Self { verbose }
    }
}

impl EventSink for CliEventSink {
    fn emit(&self, event: RuntimeEvent) {
        match event {
            RuntimeEvent::ContentDelta(text) => {
                let _ = write!(io::stdout(), "{}", text);
                let _ = io::stdout().flush();
            }
            RuntimeEvent::ReasoningDelta(text) if self.verbose => {
                let _ = write!(io::stderr(), "{}", text);
                let _ = io::stderr().flush();
            }
            RuntimeEvent::StreamError(msg) => {
                eprintln!("[stream] {}", msg);
            }
            RuntimeEvent::CompactionStarted => {
                eprintln!("[compact] summarizing conversation…");
            }
            RuntimeEvent::ContextCompacted { summary_chars } => {
                eprintln!("[compact] done (summary ~{} chars)", summary_chars);
            }
            RuntimeEvent::ToolStart { name, .. } if self.verbose => {
                eprintln!("[tool] start {}", name);
            }
            RuntimeEvent::ToolResult { name, content, .. } if self.verbose => {
                let preview: String = content.chars().take(200).collect();
                eprintln!("[tool] {} → {}", name, preview);
            }
            RuntimeEvent::Connecting {
                attempt,
                max_retries,
            } if self.verbose => {
                eprintln!("[connect] attempt {}/{}", attempt, max_retries);
            }
            RuntimeEvent::PreparingTools if self.verbose => {
                eprintln!("[stream] preparing tools…");
            }
            RuntimeEvent::BackgroundTaskResult(msg) => {
                eprintln!("[background] {}", msg);
            }
            RuntimeEvent::PlanUpdate(v) if self.verbose => {
                eprintln!("[plan] {}", v);
            }
            RuntimeEvent::SaveSession if self.verbose => {
                eprintln!("[session] save checkpoint");
            }
            _ => {}
        }
    }
}

/// In-process tool port over [`ToolRegistry`].
pub struct RegistryToolPort {
    registry: Arc<ToolRegistry>,
    agent: AgentExecutionContext,
}

impl RegistryToolPort {
    pub fn new(registry: Arc<ToolRegistry>, session_id: &str) -> Self {
        Self {
            registry,
            agent: AgentExecutionContext::root(SessionId::new(session_id)),
        }
    }
}

#[async_trait]
impl ToolPort for RegistryToolPort {
    async fn execute(&self, req: ToolRequest) -> ToolResponse {
        if req.name == "execute_command" || req.name == "exec_command" {
            if let Some(cmd) = req.arguments.get("command").and_then(|v| v.as_str()) {
                let risk = crate::runtime::guardian::classify_risk(cmd);
                if risk >= crate::runtime::guardian::RiskLevel::Critical {
                    let content = format!(
                        r#"{{"success":false,"error":"GUARDIAN BLOCK: critical-risk command rejected. {}"}}"#,
                        cmd
                    );
                    return ToolResponse {
                        content,
                        success: false,
                    };
                }
            }
        }

        let inv_id = req
            .invocation_id
            .clone()
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        let context = ToolContext {
            agent: &self.agent,
            invocation_id: ToolInvocationId::new(inv_id),
            origin_turn_id: req.turn_id.as_deref(),
            workdir: None,
        };
        match self
            .registry
            .execute_with_context(&context, &req.name, req.arguments)
            .await
        {
            Ok(result) => {
                let content = serde_json::json!({
                    "success": true,
                    "output_type": result.output_type,
                    "content": result.content,
                    "metadata": result.metadata
                })
                .to_string();
                ToolResponse {
                    content,
                    success: true,
                }
            }
            Err(e) => {
                let content = serde_json::json!({
                    "success": false,
                    "error": {
                        "message": e.message,
                        "code": e.code
                    }
                })
                .to_string();
                ToolResponse {
                    content,
                    success: false,
                }
            }
        }
    }

    fn definitions(&self) -> Vec<ToolDefinition> {
        self.registry
            .list()
            .into_iter()
            .map(|t| ToolDefinition::new(t.name(), t.description(), t.input_schema()))
            .collect()
    }
}

/// Run a single headless agent turn (tools + micro/auto compaction, streaming to stdout).
pub async fn run_oneshot(settings: Settings, prompt: String) -> anyhow::Result<()> {
    let client = ApiClient::new(settings.clone());
    if client.get_api_key().is_none() {
        anyhow::bail!(
            "API key not configured. Set ANTHROPIC_API_KEY / DEEPSEEK_API_KEY / DASHSCOPE_API_KEY \
             or run: wgenty-code config set models.main.api_key \"your-key\""
        );
    }

    // CodeGraph availability notice (silent when installed+initialized or
    // dismissed). Printed to stderr so it never pollutes query stdout.
    if let Some(msg) = crate::mcp::codegraph::install_state_notice(
        crate::mcp::codegraph::probe_install_state(&settings),
    ) {
        eprintln!("{msg}");
    }

    let session_id = Uuid::new_v4().to_string();
    let prompt_ctx = PromptContext::default()
        .with_codegraph_state(crate::mcp::codegraph::probe_install_state(&settings));
    let assembled = prompts::assemble_instructions(&settings, &prompt_ctx);
    let system_messages = assembled.system_messages.clone();
    let mut seed = assembled.system_messages;
    seed.push(ChatMessage::user(&prompt));

    // Shared memory manager: recall at start + extract during auto-compact.
    let memory_manager = Arc::new(MemoryManager::new(crate::utils::current_project_root()));
    {
        if memory_manager.load().await.is_ok() {
            let recall_top_n = settings.storage.memory.recall_top_n;
            let recall_threshold = settings.storage.memory.recall_similarity_threshold;
            crate::context::inject::MemoryContextInjector::inject(
                &mut seed,
                memory_manager.as_ref(),
                &prompt,
                recall_top_n,
                recall_threshold as f64,
            )
            .await;
        }
    }

    let history = MutexHistoryStore::new(Arc::new(Mutex::new(seed)));
    let llm = ApiLlmPort::new(client);
    // Same client for summarization (tools omitted in chat_completion call).
    let llm_for_compact: Arc<dyn crate::agent::runtime::LlmPort> = Arc::new(llm.clone());
    let registry = Arc::new(ToolRegistry::new().with_settings(&settings));
    let tools = RegistryToolPort::new(registry, &session_id);
    let events = CliEventSink::new(std::env::var("WGENTY_VERBOSE").is_ok());

    let verbose = std::env::var("WGENTY_VERBOSE").is_ok();
    let compactor = ApiCompactor::new(llm_for_compact, system_messages, Some(memory_manager))
        .with_status_sink(move |msg| {
            // Always show compact status on stderr (not only when verbose).
            eprintln!("[compact] {}", msg);
            let _ = verbose; // silence unused when not verbose-gated
        });

    let config = RuntimeConfig {
        max_rounds: settings.agent.max_rounds.unwrap_or(100),
        plan_mode: false,
        subagent_timeout_secs: settings.agent.subagent.timeout_secs,
        context_window: settings.models.context_window,
        max_tokens: settings.models.transport.max_tokens,
        session_id,
        turn_id: Some(Uuid::new_v4().to_string()),
        agent_generation: 0,
        stream_max_retries: 2,
    };

    let mut state = LoopTurnState::default();
    let _final = run_agent_loop(RunLoopArgs {
        llm: &llm,
        tools: &tools,
        events: &events,
        history: &history,
        config: &config,
        state: &mut state,
        stream_style: StreamStyle::default(),
        hooks: LoopHooks {
            compactor: Some(&compactor),
            interaction: None,
            planner: None,
            stuck_detector: None,
            token_counter: None,
            synthesis: None,
            observer: None,
            task_progress: None,
            inbox: None,
        },
    })
    .await?;

    // Content was already streamed via CliEventSink::ContentDelta.
    println!();
    Ok(())
}
