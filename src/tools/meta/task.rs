//! Task Tool — subagent spawning for complex, multi-step tasks.
//!
//! The `task` tool allows the parent agent to delegate work to an isolated
//! subagent with its own message context, filtered tool set (no recursive
//! `task` calls to prevent explosion), and a complete agent loop.
//!
//! Available subagent types:
//! - `general-purpose` (default) — general tool-use tasks
//! - `explore`                   — codebase search and analysis
//! - `plan`                      — architecture planning and breakdown

use crate::agent::progress::{ProgressCallback, SubagentProgress, SubagentStatus};
use crate::agent::{
    AgentCoordinator, ChildTerminal, CoordinatorError, SpawnChildRequest, ToolContext,
};
use crate::api::ApiClient;
use crate::config::Settings;
use crate::teams::subagent_loop::{run_subagent_loop, SubagentError};
use crate::teams::subagent_mailbox::SubagentResultMailbox;
use crate::tools::{Tool, ToolError, ToolOutput};
use crate::transcript::TranscriptStatus;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

mod heuristic;
mod transcript;

#[cfg(test)]
mod tests;

use heuristic::is_complex_task;
use transcript::build_transcript;

pub struct TaskTool {
    settings: Settings,
    tool_registry: std::sync::Weak<crate::tools::ToolRegistry>,
    /// Exclusive owner of agent spawning, concurrency, and lifecycle. The
    /// `task` tool delegates all child creation and completion to it; it never
    /// derives identity from model-supplied JSON.
    coordinator: Arc<AgentCoordinator>,
    /// Mailbox for offloading large subagent results to disk.
    mailbox: SubagentResultMailbox,
    /// Shared store for subagent progress updates (session_id → node_id → progress).
    progress_store: Arc<RwLock<HashMap<String, HashMap<String, SubagentProgress>>>>,
    /// Optional transcript store for persisting subagent execution transcripts to SQLite.
    transcript_store: Option<Arc<crate::transcript::SubagentTranscriptStore>>,
}

impl TaskTool {
    pub fn new(
        settings: Settings,
        tool_registry: std::sync::Weak<crate::tools::ToolRegistry>,
        coordinator: Arc<AgentCoordinator>,
        progress_store: Arc<RwLock<HashMap<String, HashMap<String, SubagentProgress>>>>,
        transcript_store: Option<Arc<crate::transcript::SubagentTranscriptStore>>,
    ) -> Self {
        Self {
            settings,
            tool_registry,
            coordinator,
            progress_store,
            mailbox: SubagentResultMailbox::default_location(),
            transcript_store,
        }
    }

    /// Create a ProgressCallback that writes to the shared progress store.
    fn make_progress_callback(
        store: Arc<RwLock<HashMap<String, HashMap<String, SubagentProgress>>>>,
        session_id: String,
        node_id: String,
        parent_id: Option<String>,
        label: String,
    ) -> ProgressCallback {
        Arc::new(move |mut progress: SubagentProgress| {
            progress.node_id = node_id.clone();
            progress.parent_id = parent_id.clone();
            progress.label = label.clone();
            let store = store.clone();
            let node_id = node_id.clone();
            let sid = session_id.clone();
            tokio::spawn(async move {
                let mut store = store.write().await;
                store.entry(sid).or_default().insert(node_id, progress);
            });
        })
    }
}

#[async_trait]
impl Tool for TaskTool {
    fn name(&self) -> &str {
        "task"
    }

    fn is_read_only(&self) -> bool {
        false
    }

    fn description(&self) -> &str {
        "Launch a subagent to handle complex, multi-step tasks. \
         Available types: general-purpose (default), explore (codebase search), \
         plan (architecture). Subagents have isolated context and filtered tools \
         (no recursive task spawning). \
         Use for: tasks requiring reasoning across multiple files, architecture \
         analysis, refactoring planning, debugging complex bugs, or research \
         that needs an LLM loop. \
         AVOID for: purely deterministic tasks (pattern search, statistics, \
         counting occurrences, finding simple text patterns) — use grep/glob \
         directly in parallel instead. \
         AVOID for: reading a single known file — use file_read directly. \
         AVOID for: running a single known command — use exec_command directly."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "subagent_type": {
                    "type": "string",
                    "description": "Type of subagent: general-purpose, explore, or plan",
                    "enum": ["general-purpose", "explore", "plan"]
                },
                "description": {
                    "type": "string",
                    "description": "Short (3-5 word) description of the task"
                },
                "background": {
                    "type": "boolean",
                    "description": "Run subagent concurrently inside this agent scope. The subagent is running concurrently inside this agent scope. This agent cannot terminate until the child reaches a terminal state. Default: true"
                },
                "use_small_model": {
                    "type": "boolean",
                    "description": "When true and a small model is configured, run the subagent with a smaller/cheaper model. Use for simple, self-contained tasks (e.g., reading files, searching, running a single command). Default: false"
                },
                "token_budget": {
                    "type": "integer",
                    "description": "Optional token budget in thousands (e.g., 10 = 10k tokens). 0 = use the configured default (from settings.agent.token_budget.subagent_default_k). Omit the parameter for unlimited. Default: 0"
                },
                "prompt": {
                    "type": "string",
                    "description": "The detailed task for the subagent to perform"
                },
                "comet_context": {
                    "type": "object",
                    "description": "Optional Comet workflow context for implementer subagents",
                    "properties": {
                        "change": {
                            "type": "string",
                            "description": "The name of the Comet change being worked on"
                        },
                        "task_index": {
                            "type": "integer",
                            "description": "The task index within the change (1-based)"
                        }
                    }
                }
            },
            "required": ["description", "prompt"]
        })
    }

    /// Direct (context-free) execution is rejected: `task` is identity-sensitive
    /// and requires a trusted [`ToolContext`]. This defensive path returns an
    /// error so no caller can spawn a child without the coordinator-derived
    /// agent context.
    async fn execute(&self, _input: serde_json::Value) -> Result<ToolOutput, ToolError> {
        Err(ToolError {
            message: "task requires trusted agent context".to_string(),
            code: Some("missing_agent_context".to_string()),
        })
    }

    /// Contextual execution: derive child identity, parentage, and depth from
    /// the trusted `context.agent`, ignoring all model-supplied `_`-prefixed
    /// fields. Child creation and completion go through the `AgentCoordinator`.
    async fn execute_with_context(
        &self,
        context: &ToolContext<'_>,
        input: serde_json::Value,
    ) -> Result<ToolOutput, ToolError> {
        let _subagent_type = input["subagent_type"].as_str().unwrap_or("general-purpose");
        let description = input["description"].as_str().unwrap_or("Subagent task");
        let prompt = input["prompt"].as_str().unwrap_or("");
        let background = input["background"].as_bool().unwrap_or(true);

        // Token budget: 4-level fallback per spec §3.3a.
        // 1. explicit input.token_budget (caller-explicit; 0 = unlimited stays None)
        // 2. agent.subagent.token_budget_k (subagent override)
        // 3. agent.token_budget.subagent_default_k (when > 0)
        // 4. agent.token_budget.main_k (when > 0; 0 = unlimited)
        let token_budget: Option<u64> = input
            .get("token_budget")
            .and_then(|v| v.as_u64())
            .and_then(|v| if v == 0 { None } else { Some(v) })
            .or_else(|| {
                self.settings
                    .agent
                    .subagent
                    .token_budget_k
                    .map(|v| v as u64)
            })
            .or_else(|| {
                let d = self.settings.agent.token_budget.subagent_default_k;
                if d > 0 {
                    Some(d as u64)
                } else {
                    None
                }
            })
            .or_else(|| {
                let m = self.settings.agent.token_budget.main_k;
                if m > 0 {
                    Some(m as u64)
                } else {
                    None
                }
            });

        // Extract optional Comet workflow context and build implementer prefix.
        let comet_prefix: Option<String> = input.get("comet_context").and_then(|cc| {
            let change = cc.get("change").and_then(|v| v.as_str())?;
            let task_index = cc.get("task_index").and_then(|v| v.as_i64())?;
            Some(format!(
                "You are a Comet implementer subagent working on change '{}', task #{}. Follow test-driven-development: write tests first, ensure they fail, then implement the minimum code to pass. Report your results back to the coordinator.\n\n",
                change, task_index
            ))
        });

        // Trusted session identity: derived from the execution context, never
        // from model-supplied JSON. `_session_id` in input is ignored.
        let session_id = context.agent.session_id.as_str().to_string();

        tracing::info!(
            subagent_type = _subagent_type,
            description = description,
            prompt_len = prompt.len(),
            session_id = %session_id,
            background = background,
            "TaskTool: executing subagent"
        );

        // Upgrade the Weak reference to the tool registry.
        let tool_registry = self.tool_registry.upgrade().ok_or_else(|| ToolError {
            message: "Tool registry is no longer available".to_string(),
            code: Some("registry_dropped".to_string()),
        })?;

        // Filter tools: exclude "task" when the trusted caller depth is at the
        // limit. Depth comes from the execution context, not model input.
        let depth = context.agent.depth;
        let allowed_tools: Vec<String> = tool_registry
            .list()
            .iter()
            .map(|t| t.name().to_string())
            .filter(|name| {
                if name == "task" {
                    depth < self.settings.agent.subagent.max_depth
                } else {
                    true
                }
            })
            .collect();

        // Build system prompt based on subagent type.
        let base_system_prompt: &str = match _subagent_type {
            "explore" => {
                "You are a code exploration subagent. Your role is to search \
                 and analyze codebases thoroughly.\n\n\
                 IMPORTANT — Choose your strategy based on the task type:\n\
                 - For PATTERN SEARCH tasks (e.g. 'find all .unwrap() calls', \
                   'count .clone() usages'): use grep directly with precise \
                   regex patterns. Do NOT read full files — grep gives you \
                   matching lines directly. Call grep with the exact pattern \
                   and max_results to control output size. Report counts, file \
                   locations, and representative examples.\n\
                 - For STRUCTURAL ANALYSIS tasks (e.g. 'how does module X \
                   work'): use glob to find relevant files, then file_read \
                   to understand key files, then grep for cross-references.\n\
                 - For COUNTING/STATISTICS tasks: prefer grep with \
                   files_with_matches=true first to scope the work, then \
                   detailed grep for actual matches.\n\n\
                 Key responsibilities:\n\
                 1. Search for relevant files and code patterns\n\
                 2. Read and understand code structure\n\
                 3. Analyze dependencies and relationships\n\
                 4. Report findings clearly and concisely\n\n\
                 Use search, grep, glob, and file_read tools to explore the \
                 codebase. Be thorough but efficient — focus on answering the \
                 specific question. Return a complete, self-contained result."
            }
            "plan" => {
                "You are a subagent spawned by a coordinator. The coordinator is waiting for your result. Do not attempt to coordinate other agents yourself — focus solely on your assigned task. Return a complete, self-contained result so the coordinator can proceed without follow-up questions.\n\nYou are a planning subagent. Your role is to break down complex \
                 tasks into actionable steps.\n\nKey responsibilities:\n\
                 1. Analyze task requirements\n\
                 2. Identify key files and components\n\
                 3. Break down the work into logical steps\n\
                 4. Consider dependencies, risks, and trade-offs\n\n\
                 Use file_read and search tools to understand the codebase before \
                 planning. Be thorough and structured in your analysis."
            }
            _ => {
                "You are a subagent spawned by a coordinator. The coordinator is waiting for your result. Do not attempt to coordinate other agents yourself — focus solely on your assigned task. Return a complete, self-contained result so the coordinator can proceed without follow-up questions.\n\nYou are a general-purpose subagent. Complete the assigned task \
                 efficiently using the available tools.\n\nKey responsibilities:\n\
                 1. Understand the task requirements\n\
                 2. Use appropriate tools to accomplish the task\n\
                 3. Provide clear and complete results\n\
                 4. Handle edge cases gracefully\n\n\
                 If you need to read files, search, or execute commands, use the \
                 appropriate tools. Return a complete summary of what was accomplished."
            }
        };
        let system_prompt = if let Some(ref prefix) = comet_prefix {
            format!("{}{}", prefix, base_system_prompt)
        } else {
            base_system_prompt.to_string()
        };

        // Build the full user prompt with context.
        let full_prompt = format!(
            "## Task Description\n{}\n\n## Task Details\n{}",
            description, prompt
        );

        // ── Guard: concurrency limit — queue until slot opens ───────────
        let max = self.settings.agent.subagent.max_concurrent;
        let _ = max; // retained for future acknowledgement metadata
                     // Reserve a coordinator-owned child. The coordinator enforces both depth
                     // (DepthLimitReached) and concurrency (semaphore) using the trusted
                     // caller context. `max` is recorded for the acknowledgement metadata;
                     // The legacy polling-based concurrency counter and
                     // BackgroundManager delivery are
                     // retired in favor of the coordinator owning the child lifetime.
        let reservation = self
            .coordinator
            .reserve_child(context.agent, SpawnChildRequest::new(description))
            .await
            .map_err(map_coordinator_error)?;
        let child_context = reservation.context.clone();

        // Use small model when requested and configured.
        // small_model_settings() returns a clone with main endpoint overridden
        // by models.small (or self unchanged when models.small is None).
        let use_small = input["use_small_model"].as_bool().unwrap_or(false);
        let api_client = if use_small && self.settings.models.small.is_some() {
            ApiClient::new(self.settings.small_model_settings())
        } else {
            ApiClient::new(self.settings.clone())
        };

        // Run the subagent loop (capped at 100 rounds).
        if background {
            // ── Background mode: spawn and return immediately ──────────────
            let desc = description.to_string();
            let sys_prompt = system_prompt.clone();
            let reg = tool_registry.clone();
            let tools = allowed_tools.clone();
            let api_client_bg = if use_small && self.settings.models.small.is_some() {
                ApiClient::new(self.settings.small_model_settings())
            } else {
                ApiClient::new(self.settings.clone())
            };
            let timeout_secs = self.settings.agent.subagent.timeout_secs;

            let subagent_node_id = uuid::Uuid::new_v4().to_string();
            {
                let mut store = self.progress_store.write().await;
                store.entry(session_id.clone()).or_default().insert(
                    subagent_node_id.clone(),
                    SubagentProgress {
                        node_id: subagent_node_id.clone(),
                        parent_id: None,
                        label: format!("subagent: {}", desc),
                        status: SubagentStatus::Pending,
                        round: None,
                        max_rounds: Some(100),
                        current_tool: None,
                        current_params: None,
                        action_log: Vec::new(),
                        text_snapshot: None,
                        started_at: chrono::Utc::now().timestamp_millis(),
                        elapsed_ms: 0,
                        metadata: None,
                        progress_delta: None,
                        token_budget_k: None,
                        cumulative_tokens: 0,
                        error_details: None,
                        events: Vec::new(),
                        messages: Vec::new(),
                    },
                );
            }
            let cb = Self::make_progress_callback(
                self.progress_store.clone(),
                session_id.clone(),
                subagent_node_id.clone(),
                None,
                format!("subagent: {}", desc),
            );

            let mailbox_bg = self.mailbox.clone();
            let st_bg = _subagent_type.to_string();
            let desc_full_bg = description.to_string();
            let sid_bg = session_id.clone();
            let desc_bg = desc.clone();
            let sys_prompt_bg = sys_prompt.clone();
            let prompt_bg = full_prompt.clone();
            let transcript_store_bg = self.transcript_store.clone();
            let retention_days = self.settings.storage.transcript.max_age_days;
            let started_at_bg = chrono::Utc::now().timestamp_millis();
            let bg_node_id = subagent_node_id.clone();

            // Clone full_prompt before moving it into the run_subagent_loop call
            let prompt_owned = full_prompt.clone();
            // The coordinator-owned child context moves into the spawned task so
            // the loop runs as the child agent and cancellation propagates.
            let bg_child_context = child_context.clone();
            let bg_coordinator = self.coordinator.clone();

            tokio::spawn(async move {
                let result = run_subagent_loop(
                    &api_client_bg,
                    &reg,
                    &bg_child_context,
                    &sys_prompt,
                    &prompt_owned,
                    &tools,
                    100,
                    timeout_secs,
                    Some(cb),
                    token_budget,
                )
                .await;

                let (terminal, content) = match result {
                    Ok(r) => (
                        ChildTerminal::Completed {
                            summary: r.chars().take(500).collect(),
                        },
                        r,
                    ),
                    Err(e) => (
                        ChildTerminal::Failed {
                            code: e.code().to_string(),
                            partial_result: None,
                        },
                        format!("Subagent error: {}", e),
                    ),
                };

                // Persist the child's terminal state and release its permit
                // through the coordinator. The coordinator (not the
                // BackgroundManager) owns the child lifetime.
                let _ = bg_coordinator
                    .finish_child(&bg_child_context, terminal)
                    .await;

                // Offload large results to mailbox before storing.
                let content = mailbox_bg
                    .offload_if_large(&st_bg, &desc_full_bg, &sid_bg, &content)
                    .to_content();

                let success = !content.starts_with("Subagent error:");

                // ── Save transcript ────────────────────────────────────────
                if let Some(ref store) = transcript_store_bg {
                    let retention = if retention_days > 0 {
                        Some(retention_days)
                    } else {
                        None
                    };
                    let transcript = build_transcript(
                        bg_node_id,
                        &sid_bg,
                        &desc_bg,
                        if success {
                            TranscriptStatus::Completed
                        } else {
                            TranscriptStatus::Failed
                        },
                        Some(sys_prompt_bg),
                        prompt_bg,
                        started_at_bg,
                        0, // total_tokens — not yet tracked from subagent loop
                        0, // actual_rounds
                        token_budget,
                        None,   // error_message captured in content, not individual
                        None,   // summary
                        vec![], // events — not yet tracked from subagent loop
                    );
                    let _ = store.save(&transcript, retention);
                }
            });

            Ok(ToolOutput {
                output_type: "text".to_string(),
                content: format!(
                    "[Subagent launched in background]\ntype: {}\ndescription: {}\nstatus: running\n\nThe subagent is running concurrently inside this agent scope. This agent cannot terminate until the child reaches a terminal state.",
                    _subagent_type, description
                ),
                metadata: {
                    let mut m = HashMap::new();
                    m.insert("subagent_type".to_string(), serde_json::json!(_subagent_type));
                    m.insert("description".to_string(), serde_json::json!(description));
                    m.insert("background".to_string(), serde_json::json!(true));
                    m.insert("execution_mode".to_string(), serde_json::json!("background"));
                    m.insert("routing_reason".to_string(), serde_json::json!("direct subagent (background)"));
                    m
                },
            })
        } else {
            // ── Synchronous mode: block until complete ─────────────────────
            let subagent_node_id = uuid::Uuid::new_v4().to_string();
            let started_at_sync = chrono::Utc::now().timestamp_millis();
            {
                let mut store = self.progress_store.write().await;
                store.entry(session_id.clone()).or_default().insert(
                    subagent_node_id.clone(),
                    SubagentProgress {
                        node_id: subagent_node_id.clone(),
                        parent_id: None,
                        label: format!("subagent: {}", description),
                        status: SubagentStatus::Pending,
                        round: None,
                        max_rounds: Some(100),
                        current_tool: None,
                        current_params: None,
                        action_log: Vec::new(),
                        text_snapshot: None,
                        started_at: chrono::Utc::now().timestamp_millis(),
                        elapsed_ms: 0,
                        metadata: None,
                        progress_delta: None,
                        token_budget_k: None,
                        cumulative_tokens: 0,
                        error_details: None,
                        events: Vec::new(),
                        messages: Vec::new(),
                    },
                );
            }
            let cb = Self::make_progress_callback(
                self.progress_store.clone(),
                session_id.clone(),
                subagent_node_id.clone(),
                None,
                format!("subagent: {}", description),
            );

            let (result, routing_reason) = if self.settings.agent.rlm.enabled
                && self.settings.agent.rlm.auto_routing
                && is_complex_task(&full_prompt, use_small)
            {
                let reason = format!(
                    "RLM pipeline: prompt_len={}, use_small={}",
                    full_prompt.len(),
                    use_small
                );
                tracing::info!(
                    target: "rlm",
                    phase = "auto_route",
                    reason = %reason,
                    "Complex task detected, routing to RLM pipeline"
                );
                let result = crate::tools::meta::rlm::run_rlm_pipeline(
                    &self.settings,
                    tool_registry.clone(),
                    self.coordinator.clone(),
                    &child_context,
                    description,
                    prompt,
                    Some((self.progress_store.clone(), session_id.clone())),
                    Some(subagent_node_id.clone()),
                    token_budget,
                )
                .await
                .map(|r| r.aggregated)
                .map_err(SubagentError::from);
                (result, reason)
            } else {
                let reason = "direct subagent: simple task".to_string();
                // Run as the coordinator-owned child. The child_context was
                // reserved above from the trusted caller context.
                let result = run_subagent_loop(
                    &api_client,
                    &tool_registry,
                    &child_context,
                    &system_prompt,
                    &full_prompt,
                    &allowed_tools,
                    100,
                    self.settings.agent.subagent.timeout_secs,
                    Some(cb),
                    token_budget,
                )
                .await;
                (result, reason)
            };

            // Persist the child's terminal state and release its permit through
            // the coordinator, regardless of routing path.
            let terminal = match &result {
                Ok(r) => ChildTerminal::Completed {
                    summary: r.chars().take(500).collect(),
                },
                Err(e) => ChildTerminal::Failed {
                    code: e.code().to_string(),
                    partial_result: None,
                },
            };
            let _ = self
                .coordinator
                .finish_child(&child_context, terminal)
                .await;

            // ── Save transcript on completion/failure (sync path) ──────────
            let transcript_store_sync = self.transcript_store.clone();
            let sid_sync = session_id.clone();
            let desc_sync = description.to_string();
            let sys_prompt_sync = system_prompt.clone();
            let prompt_sync = full_prompt.clone();
            let sync_node_id = subagent_node_id.clone();
            let retention_days_sync = self.settings.storage.transcript.max_age_days;

            match result {
                Ok(result) => {
                    // Offload to mailbox if result exceeds inline threshold.
                    let response = self.mailbox.offload_if_large(
                        _subagent_type,
                        description,
                        &session_id,
                        &result,
                    );

                    // Save completed transcript
                    if let Some(ref store) = transcript_store_sync {
                        let retention = if retention_days_sync > 0 {
                            Some(retention_days_sync)
                        } else {
                            None
                        };
                        let transcript = build_transcript(
                            sync_node_id,
                            &sid_sync,
                            &desc_sync,
                            TranscriptStatus::Completed,
                            Some(sys_prompt_sync),
                            prompt_sync,
                            started_at_sync,
                            0, // total_tokens
                            0, // actual_rounds
                            token_budget,
                            None,
                            Some(result.chars().take(500).collect()),
                            vec![],
                        );
                        let _ = store.save(&transcript, retention);
                    }

                    let mut metadata = HashMap::new();
                    metadata.insert(
                        "subagent_type".to_string(),
                        serde_json::json!(_subagent_type),
                    );
                    metadata.insert("description".to_string(), serde_json::json!(description));
                    metadata.insert(
                        "routing_reason".to_string(),
                        serde_json::json!(routing_reason),
                    );

                    Ok(ToolOutput {
                        output_type: "text".to_string(),
                        content: response.to_content(),
                        metadata,
                    })
                }
                Err(e) => {
                    // Save failed transcript
                    if let Some(ref store) = transcript_store_sync {
                        let retention = if retention_days_sync > 0 {
                            Some(retention_days_sync)
                        } else {
                            None
                        };
                        let transcript = build_transcript(
                            sync_node_id,
                            &sid_sync,
                            &desc_sync,
                            TranscriptStatus::Failed,
                            Some(sys_prompt_sync),
                            prompt_sync,
                            started_at_sync,
                            0,
                            0,
                            token_budget,
                            Some(e.full_message()),
                            None,
                            vec![],
                        );
                        let _ = store.save(&transcript, retention);
                    }

                    Err(ToolError {
                        message: e.full_message(),
                        code: Some(e.code().to_string()),
                    })
                }
            }
        }
    }
}

/// Maps a coordinator error to a user-facing `ToolError`.
///
/// `DepthLimitReached` surfaces the configured limit so the model can adjust;
/// `NotVisible` and other operational failures map to a stable code without
/// leaking hidden agent identifiers.
fn map_coordinator_error(e: CoordinatorError) -> ToolError {
    match e {
        CoordinatorError::DepthLimitReached { limit } => ToolError {
            message: format!(
                "Maximum subagent depth ({}) reached. Refusing to spawn deeper subagent.",
                limit
            ),
            code: Some("depth_limit_reached".to_string()),
        },
        CoordinatorError::NotVisible => ToolError {
            message: "agent is not visible from the current execution scope".to_string(),
            code: Some("not_visible".to_string()),
        },
        other => ToolError {
            message: other.to_string(),
            code: Some("coordinator_error".to_string()),
        },
    }
}
