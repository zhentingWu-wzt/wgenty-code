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
use crate::api::ApiClient;
use crate::config::Settings;
use crate::teams::subagent_loop::run_subagent_loop;
use crate::teams::subagent_mailbox::SubagentResultMailbox;
use crate::tools::{Tool, ToolError, ToolOutput};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::RwLock;
use crate::transcript::{SubagentTranscript, TranscriptStatus, SubagentEventRecord};

/// Detect whether a prompt is complex enough to warrant RLM delegation.
///
/// Uses structural analysis instead of naive keyword matching:
/// 1. Multi-step structure (numbered steps, explicit sequencing)
/// 2. File references (paths in backticks/quotes)
/// 3. Dependency declarations ("depends on", "after X completes")
/// 4. Length as a secondary signal (>1000 chars, not 500)
///
/// This avoids routing simple tasks like "create a file" through the
/// expensive RLM pipeline.
fn is_complex_task(prompt: &str, use_small_model: bool) -> bool {
    if use_small_model {
        return false; // User explicitly asked for cheap model
    }

    let prompt = prompt.trim();
    let len = prompt.len();

    // ── Structural signals (primary) ────────────────────────────────────

    // Numbered steps: "1. Refactor auth\n2. Update callers\n3. Add tests"
    let numbered_steps = {
        let mut count = 0u32;
        for line in prompt.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with(|c: char| c.is_ascii_digit())
                && trimmed.chars().find(|c| !c.is_ascii_digit()) == Some('.')
            {
                count += 1;
            }
        }
        count
    };
    if numbered_steps >= 3 {
        return true;
    }

    // File path references: `src/auth.rs`, "path/to/file", etc.
    let file_refs = prompt.matches('`').count() / 2  // paired backticks
        + prompt.matches("src/").count()
        + prompt.matches("tests/").count()
        + prompt.matches(".rs").count()
        + prompt.matches(".ts").count()
        + prompt.matches(".js").count()
        + prompt.matches(".py").count();
    if file_refs >= 3 {
        return true;
    }

    // Explicit dependency/sequencing markers — phrase-based to avoid
    // matching common words like "first" or "after" in isolation.
    let lower = prompt.to_lowercase();
    let dependency_signals = [
        "depends on",
        "must complete before",
        "after that",
        "then you should",
        "before you",
        "first you",
        "first, ",
        "second, ",
        "finally, ",
        "step by step",
        "one by one",
    ];
    let dep_hits = dependency_signals
        .iter()
        .filter(|kw| lower.contains(*kw))
        .count();
    if dep_hits >= 3 {
        return true;
    }

    // ── Length (secondary signal, raised from 500 to 1000) ──────────────
    if len > 1000 {
        // Only trigger if there are also structural indicators.
        return numbered_steps > 0 || file_refs > 0 || dep_hits > 0;
    }

    false
}

pub struct TaskTool {
    settings: Settings,
    tool_registry: std::sync::Weak<crate::tools::ToolRegistry>,
    background_manager: std::sync::Arc<crate::tools::execution::background::BackgroundManager>,
    /// Tracks currently running subagents to enforce max_concurrent limit.
    active_count: Arc<AtomicUsize>,
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
        background_manager: std::sync::Arc<crate::tools::execution::background::BackgroundManager>,
        progress_store: Arc<RwLock<HashMap<String, HashMap<String, SubagentProgress>>>>,
        transcript_store: Option<Arc<crate::transcript::SubagentTranscriptStore>>,
    ) -> Self {
        Self {
            settings,
            tool_registry,
            background_manager,
            active_count: Arc::new(AtomicUsize::new(0)),
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

/// Helper to build a SubagentTranscript from subagent execution metadata.
fn build_transcript(
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
        max_rounds: Some(30),
        actual_rounds,
        token_budget_k,
        error_message,
        summary,
        events,
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
         (no recursive task spawning). Use for: parallel work, context-heavy \
         research, complex multi-step tasks."
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
                    "description": "Run subagent in background. Returns task_id immediately; result delivered later. Default: false"
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
                }
            },
            "required": ["description", "prompt"]
        })
    }

    async fn execute(&self, input: serde_json::Value) -> Result<ToolOutput, ToolError> {
        let _subagent_type = input["subagent_type"].as_str().unwrap_or("general-purpose");
        let description = input["description"].as_str().unwrap_or("Subagent task");
        let prompt = input["prompt"].as_str().unwrap_or("");
        let background = input["background"].as_bool().unwrap_or(false);

        // Token budget: 4-level fallback per spec §3.3a.
        // 1. explicit input.token_budget (caller-explicit; 0 = unlimited stays None)
        // 2. agent.subagent.token_budget_k (subagent override)
        // 3. agent.token_budget.subagent_default_k (when > 0)
        // 4. agent.token_budget.main_k (when > 0; 0 = unlimited)
        let token_budget: Option<u64> = input.get("token_budget")
            .and_then(|v| v.as_u64())
            .and_then(|v| if v == 0 { None } else { Some(v) })
            .or_else(|| self.settings.agent.subagent.token_budget_k.map(|v| v as u64))
            .or_else(|| {
                let d = self.settings.agent.token_budget.subagent_default_k;
                if d > 0 { Some(d as u64) } else { None }
            })
            .or_else(|| {
                let m = self.settings.agent.token_budget.main_k;
                if m > 0 { Some(m as u64) } else { None }
            });

        let session_id = input["_session_id"]
            .as_str()
            .unwrap_or("default")
            .to_string();

        tracing::info!(
            subagent_type = _subagent_type,
            description = description,
            prompt_len = prompt.len(),
            session_id = %session_id,
            background = background,
            "TaskTool: executing subagent"
        );

        // Register root node in progress store.
        let root_node_id = uuid::Uuid::new_v4().to_string();
        {
            let mut store = self.progress_store.write().await;
            store.entry(session_id.clone()).or_default().insert(
                root_node_id.clone(),
                SubagentProgress {
                    node_id: root_node_id.clone(),
                    parent_id: None,
                    label: format!("task: {}", description),
                    status: SubagentStatus::Running,
                    round: None,
                    max_rounds: None,
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
                },
            );
        }

        // Upgrade the Weak reference to the tool registry.
        let tool_registry = self.tool_registry.upgrade().ok_or_else(|| ToolError {
            message: "Tool registry is no longer available".to_string(),
            code: Some("registry_dropped".to_string()),
        })?;

        // Filter tools: exclude "task" when depth exceeds limit.
        let depth = input["_subagent_depth"].as_u64().unwrap_or(0) as usize;
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
        let system_prompt = match _subagent_type {
            "explore" => {
                "You are a subagent spawned by a coordinator. The coordinator is waiting for your result. Do not attempt to coordinate other agents yourself — focus solely on your assigned task. Return a complete, self-contained result so the coordinator can proceed without follow-up questions.\n\nYou are a code exploration subagent. Your role is to search and \
                 analyze codebases thoroughly.\n\nKey responsibilities:\n\
                 1. Search for relevant files and code patterns\n\
                 2. Read and understand code structure\n\
                 3. Analyze dependencies and relationships\n\
                 4. Report findings clearly and concisely\n\n\
                 Use search, grep, glob, and file_read tools to explore the \
                 codebase. Be thorough but efficient — focus on answering the \
                 specific question."
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

        // Build the full user prompt with context.
        let full_prompt = format!(
            "## Task Description\n{}\n\n## Task Details\n{}",
            description, prompt
        );

        // ── Guard: depth limit ──────────────────────────────────────────
        let depth = input["_subagent_depth"].as_u64().unwrap_or(0) as usize;
        if depth >= self.settings.agent.subagent.max_depth {
            return Ok(ToolOutput {
                output_type: "text".to_string(),
                content: format!(
                    "Maximum subagent depth ({}) reached. Refusing to spawn deeper subagent.",
                    self.settings.agent.subagent.max_depth
                ),
                metadata: HashMap::new(),
            });
        }

        // ── Guard: concurrency limit — queue until slot opens ───────────
        let max = self.settings.agent.subagent.max_concurrent;
        let wait_start = tokio::time::Instant::now();
        const POLL_INTERVAL_MS: u64 = 250;
        const MAX_WAIT_SECS: u64 = 120;

        loop {
            let current = self.active_count.load(Ordering::SeqCst);
            if current < max {
                break;
            }
            if wait_start.elapsed().as_secs() >= MAX_WAIT_SECS {
                return Ok(ToolOutput {
                    output_type: "text".to_string(),
                    content: format!(
                        "Maximum concurrent subagents ({}) reached and queue wait expired ({} running). Try again later.",
                        max, current
                    ),
                    metadata: HashMap::new(),
                });
            }
            tokio::time::sleep(std::time::Duration::from_millis(POLL_INTERVAL_MS)).await;
        }
        self.active_count.fetch_add(1, Ordering::SeqCst);

        // Use small model when requested and configured.
        // small_model_settings() returns a clone with main endpoint overridden
        // by models.small (or self unchanged when models.small is None).
        let use_small = input["use_small_model"].as_bool().unwrap_or(false);
        let api_client = if use_small && self.settings.models.small.is_some() {
            ApiClient::new(self.settings.small_model_settings())
        } else {
            ApiClient::new(self.settings.clone())
        };

        // Run the subagent loop (capped at 30 rounds).
        if background {
            // ── Background mode: spawn and return immediately ──────────────
            let desc = description.to_string();
            let sys_prompt = system_prompt.to_string();
            let bg = self.background_manager.clone();
            let active = self.active_count.clone();
            let reg = tool_registry.clone();
            let tools = allowed_tools.clone();
            let api_client_bg = ApiClient::new(self.settings.clone());
            let timeout_secs = self.settings.agent.subagent.timeout_secs;

            let subagent_node_id = uuid::Uuid::new_v4().to_string();
            {
                let mut store = self.progress_store.write().await;
                store.entry(session_id.clone()).or_default().insert(
                    subagent_node_id.clone(),
                    SubagentProgress {
                        node_id: subagent_node_id.clone(),
                        parent_id: Some(root_node_id.clone()),
                        label: format!("subagent: {}", desc),
                        status: SubagentStatus::Pending,
                        round: None,
                        max_rounds: Some(30),
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
                    },
                );
            }
            let cb = Self::make_progress_callback(
                self.progress_store.clone(),
                session_id.clone(),
                subagent_node_id.clone(),
                Some(root_node_id.clone()),
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

            tokio::spawn(async move {
                let result = run_subagent_loop(
                    &api_client_bg,
                    &reg,
                    &sys_prompt,
                    &prompt_owned,
                    &tools,
                    30,
                    timeout_secs,
                    Some(cb),
                    token_budget,
                )
                .await;

                active.fetch_sub(1, Ordering::SeqCst);

                let (success, content) = match result {
                    Ok(r) => (true, r),
                    Err(e) => (false, format!("Subagent error: {}", e)),
                };

                // Offload large results to mailbox before storing.
                let content = mailbox_bg.offload_if_large(&st_bg, &desc_full_bg, &sid_bg, &content)
                    .to_content();

                bg.push_subagent_result(&desc, &content, success).await;

                // ── Save transcript ────────────────────────────────────────
                if let Some(ref store) = transcript_store_bg {
                    let retention = if retention_days > 0 { Some(retention_days) } else { None };
                    let transcript = build_transcript(
                        bg_node_id,
                        &sid_bg,
                        &desc_bg,
                        if success { TranscriptStatus::Completed } else { TranscriptStatus::Failed },
                        Some(sys_prompt_bg),
                        prompt_bg,
                        started_at_bg,
                        0,     // total_tokens — not yet tracked from subagent loop
                        0,     // actual_rounds
                        token_budget,
                        None,  // error_message captured in content, not individual
                        None,  // summary
                        vec![], // events — not yet tracked from subagent loop
                    );
                    let _ = store.save(&transcript, retention);
                }
            });

            Ok(ToolOutput {
                output_type: "text".to_string(),
                content: format!(
                    "[Subagent launched in background]\ntype: {}\ndescription: {}\nstatus: running\n\nThe subagent result will be delivered when it completes.",
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
                        parent_id: Some(root_node_id.clone()),
                        label: format!("subagent: {}", description),
                        status: SubagentStatus::Pending,
                        round: None,
                        max_rounds: Some(30),
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
                    },
                );
            }
            let cb = Self::make_progress_callback(
                self.progress_store.clone(),
                session_id.clone(),
                subagent_node_id.clone(),
                Some(root_node_id.clone()),
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
                    description,
                    prompt,
                    Some((self.progress_store.clone(), session_id.clone())),
                    Some(subagent_node_id.clone()),
                    token_budget,
                )
                .await
                .map(|r| r.aggregated);
                (result, reason)
            } else {
                let reason = "direct subagent: simple task".to_string();
                let result = run_subagent_loop(
                    &api_client,
                    &tool_registry,
                    system_prompt,
                    &full_prompt,
                    &allowed_tools,
                    30,
                    self.settings.agent.subagent.timeout_secs,
                    Some(cb),
                    token_budget,
                )
                .await;
                (result, reason)
            };
            self.active_count.fetch_sub(1, Ordering::SeqCst);

            // ── Save transcript on completion/failure (sync path) ──────────
            let transcript_store_sync = self.transcript_store.clone();
            let sid_sync = session_id.clone();
            let desc_sync = description.to_string();
            let sys_prompt_sync = system_prompt.to_string();
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
                        let retention = if retention_days_sync > 0 { Some(retention_days_sync) } else { None };
                        let transcript = build_transcript(
                            sync_node_id,
                            &sid_sync,
                            &desc_sync,
                            TranscriptStatus::Completed,
                            Some(sys_prompt_sync),
                            prompt_sync,
                            started_at_sync,
                            0,    // total_tokens
                            0,    // actual_rounds
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
                        let retention = if retention_days_sync > 0 { Some(retention_days_sync) } else { None };
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
                            Some(e.clone()),
                            None,
                            vec![],
                        );
                        let _ = store.save(&transcript, retention);
                    }

                    Err(ToolError {
                        message: e,
                        code: Some("subagent_error".to_string()),
                    })
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_prompt_not_complex() {
        assert!(!is_complex_task(
            "create a file called config.json with default settings",
            false
        ));
        assert!(!is_complex_task(
            "read the file src/main.rs and tell me what it does",
            false
        ));
        assert!(!is_complex_task(
            "search for the authenticate function",
            false
        ));
    }

    #[test]
    fn test_numbered_steps_is_complex() {
        let prompt = "1. Refactor the auth module\n2. Update all callers\n3. Add unit tests";
        assert!(is_complex_task(prompt, false));
    }

    #[test]
    fn test_dependency_chain_is_complex() {
        let prompt = "step by step: first, analyze the codebase, then you should identify \
                      the issues, finally, write a fix that depends on the analysis results";
        assert!(is_complex_task(prompt, false));
    }

    #[test]
    fn test_long_but_simple_not_automatically_complex() {
        let long_simple = "Please write a comprehensive explanation of how memory management \
            works in modern operating systems. Cover the basic concepts including virtual \
            memory, paging, segmentation, and how the kernel allocates and frees memory \
            for user processes. Explain the tradeoffs between different allocation \
            strategies such as best fit and first fit. Discuss how garbage collection \
            works in managed languages compared to manual memory management. Include \
            information about how modern CPUs support memory management through hardware \
            features like TLBs and page tables. Describe the role of the MMU in protecting \
            process memory spaces from each other. Provide examples of how these concepts \
            apply in practice when developing applications. Make sure to explain everything \
            clearly for someone who is new to the topic but has basic programming knowledge. \
            The explanation should be thorough but accessible and should help the reader \
            build a solid mental model of how memory management functions at both the \
            hardware and operating system levels.";
        assert!(
            long_simple.len() > 1000,
            "test precondition: text must be >1000 chars"
        );
        assert!(!is_complex_task(long_simple, false));
    }

    #[test]
    fn test_small_model_never_complex() {
        let prompt = "1. Refactor auth\n2. Update callers\n3. Add tests\n4. Update docs\n5. Deploy";
        assert!(!is_complex_task(prompt, true));
    }

    // ── token_budget extraction tests ───────────────────────────────────

    #[test]
    fn test_token_budget_schema_description_is_accurate() {
        let schema = TaskTool::new(
            Settings::default(),
            std::sync::Weak::new(),
            std::sync::Arc::new(crate::tools::execution::background::BackgroundManager::new()),
            std::sync::Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
            None, // transcript_store
        ).input_schema();
        let desc = schema["properties"]["token_budget"]["description"]
            .as_str()
            .unwrap();
        // Must NOT claim "0 = unlimited" since 0 → fallback to settings default.
        assert!(
            desc.contains("configured default"),
            "Description should say '0 = use the configured default', got: '{}'",
            desc
        );
        // Must mention how to get true unlimited.
        assert!(
            desc.contains("omit") || desc.contains("Omit"),
            "Description should mention 'Omit the parameter for unlimited', got: '{}'",
            desc
        );
    }

    #[test]
    fn test_token_budget_zero_is_unlimited() {
        // token_budget=0 must produce None (unlimited), not Some(0) which
        // immediately triggers budget exceeded in the subagent loop.
        let default_k = 0u64;
        let input = serde_json::json!({"token_budget": 0});
        let result: Option<u64> = input.get("token_budget")
            .and_then(|v| v.as_u64())
            .and_then(|v| if v == 0 { None } else { Some(v) })
            .or_else(|| if default_k > 0 { Some(default_k) } else { None });
        assert_eq!(result, None, "token_budget=0 should produce None (unlimited)");
    }

    #[test]
    fn test_token_budget_positive_is_preserved() {
        let default_k = 0u64;
        let input = serde_json::json!({"token_budget": 10});
        let result: Option<u64> = input.get("token_budget")
            .and_then(|v| v.as_u64())
            .and_then(|v| if v == 0 { None } else { Some(v) })
            .or_else(|| if default_k > 0 { Some(default_k) } else { None });
        assert_eq!(result, Some(10), "token_budget=10 should produce Some(10)");
    }

    #[test]
    fn test_token_budget_missing_defaults_to_none() {
        let default_k = 0u64;
        let input = serde_json::json!({"prompt": "hello"});
        let result: Option<u64> = input.get("token_budget")
            .and_then(|v| v.as_u64())
            .and_then(|v| if v == 0 { None } else { Some(v) })
            .or_else(|| if default_k > 0 { Some(default_k) } else { None });
        assert_eq!(result, None, "missing token_budget with no default should produce None");
    }

    #[test]
    fn test_token_budget_uses_settings_default_when_missing() {
        let default_k = 20u64;
        let input = serde_json::json!({"prompt": "hello"});
        let result: Option<u64> = input.get("token_budget")
            .and_then(|v| v.as_u64())
            .and_then(|v| if v == 0 { None } else { Some(v) })
            .or_else(|| if default_k > 0 { Some(default_k) } else { None });
        assert_eq!(result, Some(20), "missing token_budget with default=20 should produce Some(20)");
    }

    #[test]
    fn test_token_budget_zero_with_nonzero_default_falls_back_to_default() {
        // When token_budget=0 is explicit but settings has a non-zero default,
        // the 0→None mapping makes or_else pick up the default.
        let default_k = 20u64;
        let input = serde_json::json!({"token_budget": 0});
        let result: Option<u64> = input.get("token_budget")
            .and_then(|v| v.as_u64())
            .and_then(|v| if v == 0 { None } else { Some(v) })
            .or_else(|| if default_k > 0 { Some(default_k) } else { None });
        assert_eq!(result, Some(20), "explicit token_budget=0 with non-zero default should use the default");
    }
}
