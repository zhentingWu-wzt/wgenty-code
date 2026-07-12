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

use crate::agent::progress::{ErrorType, ProgressCallback, SubagentProgress, SubagentStatus};
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
use futures::FutureExt;
use std::collections::HashMap;
use std::panic::AssertUnwindSafe;
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
        // `background` is a legacy mode switch that is now ignored: every
        // `task` call spawns asynchronously. Track whether the model supplied
        // it so the acknowledgement can report it as ignored rather than
        // silently dropping a field the model thinks it controls.
        let background_supplied = input.get("background").is_some();

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
            background_supplied = background_supplied,
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
        let _ = self.settings.agent.subagent.max_concurrent; // retained for acknowledgement metadata
                                                             // ── Unified asynchronous path ───────────────────────────────────
                                                             // `task` always spawns a coordinator-owned child and returns a
                                                             // structured acknowledgement immediately. The legacy sync/background
                                                             // split is retired: a parent observes child results through the
                                                             // coordinator (non-root parents get a synthesis round; the persistent
                                                             // root consumes ready groups via the daemon delivery API).
        let is_root_call = context.agent.parent_id.is_none();
        let group_id = if is_root_call {
            let turn_id = context.origin_turn_id.ok_or_else(|| ToolError {
                message: "root task invocation is missing its trusted turn id".to_string(),
                code: Some("missing_turn_context".to_string()),
            })?;
            self.coordinator
                .current_or_create_root_group(context.agent, turn_id)
                .await
                .map_err(map_coordinator_error)?
        } else {
            self.coordinator
                .current_or_create_parent_group(context.agent)
                .await
                .map_err(map_coordinator_error)?
        };

        let reservation = self
            .coordinator
            .reserve_child_in_group(
                context.agent,
                SpawnChildRequest::new(description),
                group_id.clone(),
            )
            .await
            .map_err(map_coordinator_error)?;
        let child_context = reservation.context.clone();
        let child_id = child_context.agent_id.clone();

        // Use small model when requested and configured.
        // small_model_settings() returns a clone with main endpoint overridden
        // by models.small (or self unchanged when models.small is None).
        let use_small = input["use_small_model"].as_bool().unwrap_or(false);
        let api_client = if use_small && self.settings.models.small.is_some() {
            ApiClient::new(self.settings.small_model_settings())
        } else {
            ApiClient::new(self.settings.clone())
        };

        // ── Unified spawn: progress record + spawned child future ──────
        let timeout_secs = self.settings.agent.subagent.timeout_secs;
        let mailbox_bg = self.mailbox.clone();
        let st_bg = _subagent_type.to_string();
        let desc_full_bg = description.to_string();
        let sid_bg = session_id.clone();
        let desc_bg = description.to_string();
        let sys_prompt_bg = system_prompt.clone();
        let prompt_bg = full_prompt.clone();
        let transcript_store_bg = self.transcript_store.clone();
        let retention_days = self.settings.storage.transcript.max_age_days;
        let started_at_bg = chrono::Utc::now().timestamp_millis();
        let subagent_node_id = child_context.agent_id.as_str().to_string();
        let bg_node_id = subagent_node_id.clone();
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

        let reg = tool_registry.clone();
        let tools = allowed_tools.clone();
        let rlm_enabled = self.settings.agent.rlm.enabled
            && self.settings.agent.rlm.auto_routing
            && is_complex_task(&full_prompt, use_small);
        let progress_store_bg = self.progress_store.clone();
        let sid_for_rlm = session_id.clone();
        let node_id_for_rlm = subagent_node_id.clone();
        let settings_bg = self.settings.clone();
        let coordinator_bg = self.coordinator.clone();

        // The coordinator-owned child context moves into the spawned task so
        // the loop runs as the child agent and cancellation propagates. The
        // spawned future persists its own terminal through the coordinator so
        // root children become deliverable even when no parent joins them.
        let bg_child_context = child_context.clone();
        let handle = tokio::spawn(async move {
            // Wrap the subagent loop in `catch_unwind` so a panic inside the
            // loop is converted to a `SubagentError` instead of aborting the
            // spawned future before `finish_child` can run. Without this, a
            // panicking child would leave its task-group slot unfilled forever,
            // causing the parent agent to wait for a delivery that never comes.
            let loop_future = async {
                if rlm_enabled {
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
                    crate::tools::meta::rlm::run_rlm_pipeline(
                        &settings_bg,
                        reg.clone(),
                        coordinator_bg.clone(),
                        &bg_child_context,
                        &desc_full_bg,
                        &prompt_bg,
                        Some((progress_store_bg, sid_for_rlm)),
                        Some(node_id_for_rlm),
                        token_budget,
                    )
                    .await
                    .map(|r| r.aggregated)
                    .map_err(SubagentError::from)
                } else {
                    run_subagent_loop(
                        &api_client,
                        &reg,
                        &bg_child_context,
                        coordinator_bg.clone(),
                        &sys_prompt_bg,
                        &prompt_bg,
                        &tools,
                        100,
                        timeout_secs,
                        Some(cb),
                        token_budget,
                    )
                    .await
                }
            };

            let result = match AssertUnwindSafe(loop_future).catch_unwind().await {
                Ok(r) => r,
                Err(_) => Err(SubagentError {
                    message: "subagent panicked during execution".to_string(),
                    error_type: ErrorType::Unknown,
                    partial_result: None,
                }),
            };

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
            // through the coordinator. The coordinator (not a background
            // manager) owns the child lifetime. If `finish_child` fails (e.g.,
            // store I/O error), fall back to `force_record_child_result` so the
            // group result is still recorded and the parent agent receives the
            // delivery instead of waiting forever.
            if let Err(error) = coordinator_bg
                .finish_child(&bg_child_context, terminal.clone())
                .await
            {
                tracing::error!(
                    child_id = %bg_child_context.agent_id,
                    error = %error,
                    "failed to persist child terminal state; attempting fallback"
                );
                coordinator_bg
                    .force_record_child_result(&bg_child_context, &terminal)
                    .await;
            }

            // Offload large results to mailbox before storing transcript.
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
                    0, // total_tokens - not yet tracked from subagent loop
                    0, // actual_rounds
                    token_budget,
                    None,   // error_message captured in content, not individual
                    None,   // summary
                    vec![], // events - not yet tracked from subagent loop
                );
                let _ = store.save(&transcript, retention);
            }

            terminal
        });

        // The coordinator owns the spawned handle so finalization, joining,
        // and cancellation can await or abort it. Registration cannot fail
        // for a freshly reserved child scope.
        if let Err(error) = self.coordinator.register_task(&child_context, handle).await {
            tracing::error!(
                child_id = %child_id,
                error = %error,
                "failed to register child task handle with coordinator"
            );
        }

        // ── Return one structured acknowledgement ───────────────────────
        let mut metadata = HashMap::from([
            ("child_id".to_string(), serde_json::json!(child_id.as_str())),
            (
                "task_group_id".to_string(),
                serde_json::json!(group_id.as_str()),
            ),
            ("status".to_string(), serde_json::json!("running")),
            (
                "subagent_type".to_string(),
                serde_json::json!(_subagent_type),
            ),
            ("description".to_string(), serde_json::json!(description)),
        ]);
        // Compatibility: report the legacy `background` switch as ignored when
        // the model supplies it, instead of branching on its value.
        if background_supplied {
            metadata.insert(
                "ignored_arguments".to_string(),
                serde_json::json!(["background"]),
            );
        }
        Ok(ToolOutput {
            output_type: "json".to_string(),
            content: serde_json::json!({
                "child_id": child_id.as_str(),
                "task_group_id": group_id.as_str(),
                "status": "running"
            })
            .to_string(),
            metadata,
        })
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
