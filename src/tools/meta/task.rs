//! Task Tool — subagent spawning for complex, multi-step tasks.
//!
//! The `task` tool allows the parent agent to delegate work to an isolated
//! subagent with its own message context, filtered tool set, and a complete
//! agent loop. Explore/plan are leaf types (no spawn tools). General-purpose
//! may attempt nested `task` calls; depth is enforced by the coordinator, and
//! blocked deeper spawns self-execute in the non-root parent (structural
//! fallback) so the delegated work is not dropped.
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
use crate::config::agent::RootPermissionMode;
use crate::config::Settings;
use crate::permissions::policy::ToolPermissionPolicy;
use crate::runtime::guardian::Guardian;
use crate::teams::guarding_tool_port::SubagentPermissionContext;
use crate::teams::permission_bridge::PermissionBridge;
use crate::teams::subagent_loop::{run_subagent_loop_with_permissions, SubagentError};
use crate::teams::subagent_mailbox::SubagentResultMailbox;
use crate::tools::{Tool, ToolError, ToolOutput};
use crate::transcript::TranscriptStatus;
use async_trait::async_trait;
use futures::FutureExt;
use std::collections::{HashMap, HashSet};
use std::panic::AssertUnwindSafe;
use std::sync::{Arc, Mutex};
use tokio::sync::RwLock;

mod heuristic;
mod routing;
pub(crate) mod transcript;

use self::transcript::{new_transcript_id, save_minimal_transcript};

/// Convert a live `SubagentEvent` (progress timeline) into the persisted
/// `SubagentEventRecord` (transcript) shape.
fn convert_event(
    e: &crate::agent::progress::SubagentEvent,
) -> crate::transcript::SubagentEventRecord {
    use crate::agent::progress::SubagentEventType;
    let (event_type, tool_name, data) = match &e.event_type {
        SubagentEventType::Thought { text } => ("thought".to_string(), None, text.clone()),
        SubagentEventType::Action {
            tool_name,
            params_summary,
        } => (
            "action".to_string(),
            Some(tool_name.clone()),
            params_summary.clone(),
        ),
        SubagentEventType::ToolResult {
            tool_name,
            success,
            summary,
        } => (
            "tool_result".to_string(),
            Some(tool_name.clone()),
            format!("success={success} {summary}"),
        ),
        SubagentEventType::Error { message, .. } => ("error".to_string(), None, message.clone()),
        SubagentEventType::Completion { status, summary } => (
            "completion".to_string(),
            None,
            format!("{status} {}", summary.as_deref().unwrap_or("")),
        ),
        SubagentEventType::Permission { kind, detail } => {
            ("permission".to_string(), None, format!("{kind}: {detail}"))
        }
    };
    crate::transcript::SubagentEventRecord {
        round: 0,
        event_type,
        tool_name,
        tool_params: None,
        data,
        elapsed_ms: e.elapsed_ms,
        token_count: None,
    }
}

#[cfg(test)]
mod tests;

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
    /// Optional shared approval bridge for subagent policy Ask.
    permission_bridge: Option<Arc<PermissionBridge>>,
    /// Optional shared session rules with the root ToolExecutor.
    session_rules: Option<Arc<RwLock<HashSet<String>>>>,
    /// Shared root agent permission mode (Yolo/AcceptEdits/Normal).
    /// Subagents snapshot the current value at spawn time. Uses std::sync so
    /// `build_permission_context` can read it without an async context.
    root_mode: Arc<std::sync::RwLock<RootPermissionMode>>,
    /// Sandbox effective mode (includes Plan). Snapshotted into
    /// SubagentPermissionContext at spawn.
    effective_mode: Arc<std::sync::RwLock<crate::sandbox::EffectiveMode>>,
    /// Daemon-only binding for the code-selected RootCause route. The handle
    /// remains empty for normal registry/test construction.
    root_cause_runtime: crate::exec_session::RootCauseRuntimeHandle,
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
            permission_bridge: None,
            session_rules: None,
            root_mode: Arc::new(std::sync::RwLock::new(RootPermissionMode::Normal)),
            effective_mode: Arc::new(std::sync::RwLock::new(
                crate::sandbox::EffectiveMode::Normal,
            )),
            root_cause_runtime: crate::exec_session::root_cause_runtime_handle(),
        }
    }

    pub fn with_permission_bridge(mut self, bridge: Arc<PermissionBridge>) -> Self {
        self.permission_bridge = Some(bridge);
        self
    }

    pub fn with_session_rules(mut self, rules: Arc<RwLock<HashSet<String>>>) -> Self {
        self.session_rules = Some(rules);
        self
    }

    /// Set the shared root permission mode signal. The TUI/daemon updates this
    /// at runtime; each subagent snapshots the current value at spawn time.
    pub fn with_root_mode(mut self, mode: Arc<std::sync::RwLock<RootPermissionMode>>) -> Self {
        self.root_mode = mode;
        self
    }

    /// Share the daemon's effective mode lock (includes Plan).
    pub fn with_effective_mode(
        mut self,
        mode: Arc<std::sync::RwLock<crate::sandbox::EffectiveMode>>,
    ) -> Self {
        self.effective_mode = mode;
        self
    }

    /// Shares the daemon's late-bound static RootCause route runtime.
    pub fn with_root_cause_runtime(
        mut self,
        runtime: crate::exec_session::RootCauseRuntimeHandle,
    ) -> Self {
        self.root_cause_runtime = runtime;
        self
    }

    /// Update the root permission mode at runtime.
    pub fn set_root_mode(&self, mode: RootPermissionMode) {
        *self.root_mode.write().unwrap() = mode;
    }

    fn build_permission_context(&self, agent_id: &str) -> SubagentPermissionContext {
        let workspace = self.settings.storage.working_dir.clone();
        let limits = &self.settings.agent.subagent;
        let root_mode = *self.root_mode.read().unwrap_or_else(|e| e.into_inner());
        let effective_mode = *self
            .effective_mode
            .read()
            .unwrap_or_else(|e| e.into_inner());
        SubagentPermissionContext {
            policy: ToolPermissionPolicy::new(workspace),
            session_rules: self
                .session_rules
                .clone()
                .unwrap_or_else(|| Arc::new(RwLock::new(HashSet::new()))),
            bridge: self.permission_bridge.clone(),
            ask_strategy: limits.ask_strategy,
            approval_timeout_secs: limits.approval_timeout_secs,
            timeout_decision: limits.timeout_decision,
            guardian: Guardian::default(),
            agent_id: agent_id.to_string(),
            root_mode,
            effective_mode,
            denial_log: Arc::new(Mutex::new(Vec::new())),
            event_log: Arc::new(Mutex::new(Vec::new())),
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

    /// Interception point 1: synchronous fallback execution inside TaskTool.
    ///
    /// Called when `reserve_child_in_group` fails with a structural
    /// `CoordinatorError` (`DepthLimitReached` / `ConcurrencyClosed` / `TaskGroup`).
    /// Runs `full_prompt` using the parent agent's api client and tool registry,
    /// returning the result as a `ToolOutput`. The parent agent model is unaware
    /// that a fallback occurred.
    ///
    /// Guards:
    /// - `!is_root_caller(context.agent)` (root must not self-execute; Comet isolation)
    /// - `!fallback_already_used` (single-shot, non-recursive constraint)
    ///
    /// The fallback loop is intentionally a leaf execution: spawn tools
    /// (`task` / `delegate`) are stripped so the synthetic ghost agent cannot
    /// re-enter nested dispatch (it is not registered in coordinator scopes).
    ///
    /// On fallback execution failure: returns `ToolError` (degrades to root model,
    /// no recursion). Structural failures do not swap the model.
    async fn execute_fallback_sync(
        &self,
        context: &ToolContext<'_>,
        description: &str,
        full_prompt: &str,
        system_prompt: &str,
        fallback_key: &str,
    ) -> Result<ToolOutput, ToolError> {
        use crate::agent::fallback::{prepare_structural_fallback, FallbackBlocked, FallbackKind};

        tracing::info!(
            fallback = "interception1",
            kind = ?FallbackKind::Structural,
            description = %description,
            "Subagent dispatch fallback: synchronous execution in TaskTool"
        );

        // Shared guard + ghost-context preparation (root-caller rejection +
        // single-shot constraint), marked used BEFORE execution to prevent
        // re-entry. See `agent::fallback::prepare_structural_fallback`.
        let prepared =
            match prepare_structural_fallback(&self.coordinator, context.agent, fallback_key).await
            {
                Ok(p) => p,
                Err(FallbackBlocked::RootCaller) => {
                    return Err(ToolError {
                        message: "fallback unavailable: root caller cannot self-execute"
                            .to_string(),
                        code: Some("fallback_root_blocked".to_string()),
                    });
                }
                Err(FallbackBlocked::AlreadyUsed) => {
                    return Err(ToolError {
                        message: "fallback already used for this child".to_string(),
                        code: Some("fallback_already_used".to_string()),
                    });
                }
            };

        // Structural failure does not swap the model: reuse parent's settings.
        let api_client = ApiClient::new(self.settings.clone());
        let tool_registry = self.tool_registry.upgrade().ok_or_else(|| ToolError {
            message: "tool registry unavailable for fallback".to_string(),
            code: Some("fallback_no_registry".to_string()),
        })?;

        // Ghost leaf context (`parent_id = None`) so SubagentSynthesis treats
        // this loop as a root-like leaf and skips collect_children_for_synthesis.
        // The ghost is not registered in coordinator scopes; a non-root
        // parent_id would make synthesis call active_owner_status -> NotVisible
        // and abort the work.
        let child_context = prepared.ghost;
        let child_agent_id_str = prepared.agent_id;

        // Leaf tool set only: stripping spawn tools forces the fallback to
        // complete the delegated work itself (depth-limit takeover).
        let allowed_tools: Vec<String> = tool_registry
            .list()
            .iter()
            .map(|t| t.name().to_string())
            .filter(|name| name != "task" && name != "delegate")
            .collect();
        let timeout_secs = self.settings.agent.subagent.timeout_secs;
        let workdir: Option<std::path::PathBuf> = Some(self.settings.storage.working_dir.clone());
        let permission = self.build_permission_context(&child_agent_id_str);

        let started_at = chrono::Utc::now().timestamp_millis();
        let result = run_subagent_loop_with_permissions(
            &api_client,
            tool_registry.clone(),
            &child_context,
            self.coordinator.clone(),
            system_prompt,
            full_prompt,
            &allowed_tools,
            self.settings.agent.subagent.max_rounds.unwrap_or(100),
            timeout_secs,
            None,
            None,
            workdir,
            permission,
            Arc::new(self.settings.clone()),
            self.transcript_store.clone(),
            context.origin_turn_id.map(|s| s.to_string()),
        )
        .await;

        // Best-effort transcript save for the fallback path (the main path
        // saves via the progress-store observer; fallback has no observer).
        if let Some(ref store) = self.transcript_store {
            let retention = if self.settings.storage.transcript.max_age_days > 0 {
                Some(self.settings.storage.transcript.max_age_days)
            } else {
                None
            };
            save_minimal_transcript(
                store,
                &new_transcript_id(),
                context.agent.session_id.as_str(),
                description,
                Some(system_prompt.to_string()),
                full_prompt.to_string(),
                started_at,
                &result,
                self.settings.agent.subagent.trace.context_char_limit,
                retention,
            );
        }

        match result {
            Ok(output) => {
                tracing::info!(
                    fallback = "interception1",
                    result = "success",
                    "Subagent dispatch fallback succeeded"
                );
                Ok(ToolOutput::text(output))
            }
            Err(e) => {
                tracing::warn!(
                    fallback = "interception1",
                    result = "failure",
                    error = %e.full_message(),
                    "Subagent dispatch fallback failed; degrading to root model"
                );
                Err(ToolError {
                    message: format!(
                        "Fallback execution failed: {}. Original dispatch error preserved.",
                        e.full_message()
                    ),
                    code: Some("fallback_execution_failed".to_string()),
                })
            }
        }
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
         plan (architecture), root-cause (read-only diagnosis). Subagents have isolated context and filtered tools. \
         Explore/plan never spawn further agents. General-purpose may call task \
         again; if depth limit blocks the nested spawn, the system self-executes \
         that delegated prompt in the calling subagent (structural fallback) so \
         the work is not dropped. \
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
                    "description": "Type of subagent: general-purpose, explore, plan, or root-cause",
                    "enum": ["general-purpose", "explore", "plan", "root-cause"]
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
                "isolation": {
                    "type": "string",
                    "enum": ["shared", "worktree"],
                    "description": "Working directory isolation: 'shared' (default, main checkout) or 'worktree' (dedicated git worktree on a new branch). Worktree isolation forces serial execution; the subagent is told to operate within the worktree path using absolute paths."
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
        // Map the model-supplied type string to a trusted NodeType and resolve
        // its NodeContract. The contract is the single source of truth for the
        // system prompt, allowed tools, and budget; the coordinator enforces the
        // same contract (two-layer split). The model JSON never reaches
        // SpawnChildRequest directly — it flows through parse_node_type.
        let node_type = parse_node_type(_subagent_type);
        let contract = self
            .coordinator
            .node_contract(&node_type)
            .ok_or_else(|| ToolError {
                message: format!("no NodeContract registered for {:?} node type", node_type),
                code: Some("unknown_node_type".to_string()),
            })?;
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
            .filter(|&v| v != 0)
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
        // s12 worktree isolation: optional dedicated checkout for this subagent.
        let isolation = input["isolation"].as_str().unwrap_or("shared");
        let worktree_guard: Option<crate::teams::WorktreeIsolation> = if isolation == "worktree" {
            let repo_root =
                std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
            match crate::teams::WorktreeIsolation::create(
                &repo_root,
                context.agent.agent_id.as_str(),
                None,
            ) {
                Ok(wt) => {
                    tracing::info!(
                        worktree = %wt.path.display(),
                        branch = %wt.branch,
                        "task: created worktree isolation"
                    );
                    Some(wt)
                }
                Err(e) => {
                    tracing::warn!(error = %e, "task: worktree creation failed; falling back to shared");
                    None
                }
            }
        } else {
            None
        };
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

        // Filter tools for the *spawned child* (not the caller). Depth comes
        // from the trusted execution context, never model input.
        //
        // `explore` and `plan` subagents never spawn children: recursive
        // `task`/`delegate` calls make a root task-group's ready condition
        // depend on every transitively-spawned descendant terminating, so one
        // slow/stuck grandchild blocks the parent's delivery indefinitely.
        // Explore is a leaf search agent and plan is a leaf analysis agent --
        // give them only leaf tools.
        //
        // `general-purpose` keeps spawn tools even at `max_depth`. The
        // coordinator hard-rejects deeper reserves with `DepthLimitReached`,
        // and interception-point 1 then self-executes the delegated prompt in
        // the non-root parent (depth-limit takeover). Soft-stripping `task` at
        // the limit would make that fallback path unreachable.
        let allowed_tools: Vec<String> = filter_allowed_tools(
            tool_registry.list().iter().map(|t| t.name().to_string()),
            contract,
        );

        // System prompt comes from the NodeContract (migrated from the old
        // hardcoded match arms; the contract holds the identical strings).
        let base_system_prompt: &str = contract.system_prompt.as_str();
        let system_prompt = if let Some(ref prefix) = comet_prefix {
            format!("{}{}", prefix, base_system_prompt)
        } else {
            base_system_prompt.to_string()
        };

        // Build the full user prompt with context.
        // If isolated, tell the subagent where to work (absolute paths).
        let worktree_note = worktree_guard.as_ref().map(|wt| {
            format!(
                "\n\n<worktree-isolation>\nYou are running in an isolated git worktree at: {}\nAll file operations MUST use absolute paths within this directory. Do not touch the main checkout. When done, summarize changes; the parent will merge or discard.\n</worktree-isolation>\n",
                wt.path.display()
            )
        }).unwrap_or_default();
        let full_prompt = format!(
            "## Task Description\n{}\n\n## Task Details\n{}{}",
            description, prompt, worktree_note
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

        let reservation = match self
            .coordinator
            .reserve_child_in_group(
                context.agent,
                SpawnChildRequest::new(description).with_node_type(node_type.clone()),
                group_id.clone(),
            )
            .await
        {
            Ok(r) => r,
            Err(e) => {
                // Interception point 1: pre-dispatch structural failure.
                use crate::agent::fallback::fallback_eligible_from_coordinator_error;
                if fallback_eligible_from_coordinator_error(&e).is_some() {
                    let fallback_key = format!("pending:{}", description);
                    return self
                        .execute_fallback_sync(
                            context,
                            description,
                            &full_prompt,
                            &system_prompt,
                            &fallback_key,
                        )
                        .await;
                }
                // Not eligible -> original error path.
                return Err(map_coordinator_error(e));
            }
        };
        let child_context = reservation.context.clone();
        let child_id = child_context.agent_id.clone();

        // Resolve the model for this subagent.
        //
        // Priority chain (see `routing::decide`):
        //   1. Explicit `use_small_model` from the caller LLM → Light/Heavy
        //      (preserves the pre-routing binary behavior exactly).
        //   2. Absent hint + auto-routing enabled → heuristic complexity score
        //      picks a tier (Light/Medium/Heavy), with optional LLM fallback
        //      for borderline scores.
        //   3. Routing disabled / no tier profiles / LLM fails → main model.
        //
        // `use_small_model` is read as Option<bool> so we can distinguish
        // "absent" (eligible for auto-routing) from "explicitly false".
        let use_small_model_opt = input.get("use_small_model").and_then(|v| v.as_bool());
        let models = &self.settings.models;
        let routing_cfg = &models.routing;
        let has_light = models.has_tier(crate::config::models::ModelTier::Light);
        let has_heavy = models.has_tier(crate::config::models::ModelTier::Heavy);

        let choice = routing::decide(
            use_small_model_opt,
            &full_prompt,
            routing_cfg,
            has_light,
            has_heavy,
        );

        let tier = match choice {
            routing::ModelChoice::NeedsLlm { .. } if routing_cfg.llm_fallback => {
                // Borderline + LLM fallback enabled: ask the small model for a
                // sharper score. On any failure, fall back to Medium.
                let classifier_client = ApiClient::new(self.settings.small_model_settings());
                match crate::tools::meta::rlm::classifier::classify_complexity(
                    &classifier_client,
                    &full_prompt,
                )
                .await
                {
                    Ok(score) => {
                        if score < routing_cfg.boundary_low && has_light {
                            crate::config::models::ModelTier::Light
                        } else if score > routing_cfg.boundary_high && has_heavy {
                            crate::config::models::ModelTier::Heavy
                        } else {
                            crate::config::models::ModelTier::Medium
                        }
                    }
                    Err(e) => {
                        tracing::debug!(
                            target: "routing",
                            error = %e,
                            "LLM complexity classifier failed, falling back to Medium"
                        );
                        crate::config::models::ModelTier::Medium
                    }
                }
            }
            other => other.tier_or_medium(),
        };

        let tier_endpoint = models.endpoint_for_tier(tier);
        tracing::debug!(
            target: "routing",
            tier = ?tier,
            model = %tier_endpoint.name,
            explicit = ?use_small_model_opt,
            "subagent model routed"
        );

        // Build a Settings clone with `models.main` set to the chosen tier's
        // endpoint, mirroring how `small_model_settings()` worked but for any
        // tier. This keeps every downstream path (subagent loop, fallback,
        // transcript) reading the correct model from `settings.models.main`.
        let mut child_settings = self.settings.clone();
        child_settings.models.main = tier_endpoint;
        let api_client = ApiClient::new(child_settings.clone());

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
        let progress_store_for_transcript = self.progress_store.clone();
        // `settings_bg` mirrors the child's actual model so interception-2
        // fallback reads the correct `failed_model` and `select_fallback_model`
        // does not pick the same model. `child_settings` already has
        // `models.main` set to the routed tier endpoint, so reuse it directly.
        let settings_bg = child_settings.clone();
        let coordinator_bg = self.coordinator.clone();
        let permission_ctx = self.build_permission_context(child_context.agent_id.as_str());
        // Fold subagent file edits into the root turn's checkpoint snapshot.
        let origin_turn_id_bg = context.origin_turn_id.map(|s| s.to_string());

        // The coordinator-owned child context moves into the spawned task so
        // the loop runs as the child agent and cancellation propagates. The
        // spawned future persists its own terminal through the coordinator so
        // root children become deliverable even when no parent joins them.
        let bg_child_context = child_context.clone();
        let worktree_guard_bg = worktree_guard;
        // Bind the code-owned diagnostic route after the coordinator has
        // reserved a trusted child identity, but *before* spawning its future.
        // A RootCause child can publish immediately on its first poll, so
        // binding after the task acknowledgement is a real TOCTOU bug.
        let root_cause_route_bound =
            if is_root_call && node_type == crate::org_graph::NodeType::RootCause {
                let runtime = self
                    .root_cause_runtime
                    .read()
                    .map_err(|error| ToolError {
                        message: format!("root-cause runtime handle lock: {error}"),
                        code: Some("root_cause_dispatch_failed".into()),
                    })?
                    .clone();
                if let Some(runtime) = runtime {
                    match runtime.try_bind_root_cause_child(
                        &child_context.session_id,
                        child_id.as_str().to_string(),
                    ) {
                        Ok(bound) => bound,
                        Err(error) => {
                            self.coordinator
                                .finish_child(&child_context, ChildTerminal::Cancelled)
                                .await
                                .map_err(map_coordinator_error)?;
                            return Err(ToolError {
                                message: format!("bind static root-cause child: {error:#}"),
                                code: Some("root_cause_dispatch_failed".into()),
                            });
                        }
                    }
                } else {
                    false
                }
            } else {
                false
            };
        let root_cause_runtime_bg = if root_cause_route_bound {
            self.root_cause_runtime
                .read()
                .map_err(|error| ToolError {
                    message: format!("root-cause runtime handle lock: {error}"),
                    code: Some("root_cause_dispatch_failed".into()),
                })?
                .clone()
        } else {
            None
        };
        let root_cause_session_bg = child_context.session_id.clone();
        let root_cause_child_id_bg = child_id.as_str().to_string();

        let handle = tokio::spawn(async move {
            let _worktree_guard = worktree_guard_bg;
            // Wrap the subagent loop in `catch_unwind` so a panic inside the
            // loop is converted to a `SubagentError` instead of aborting the
            // spawned future before `finish_child` can run. Without this, a
            // panicking child would leave its task-group slot unfilled forever,
            // causing the parent agent to wait for a delivery that never comes.
            let loop_future = async {
                let workdir = _worktree_guard.as_ref().map(|w| w.path.clone());
                run_subagent_loop_with_permissions(
                    &api_client,
                    reg.clone(),
                    &bg_child_context,
                    coordinator_bg.clone(),
                    &sys_prompt_bg,
                    &prompt_bg,
                    &tools,
                    100,
                    timeout_secs,
                    Some(cb),
                    token_budget,
                    workdir,
                    permission_ctx,
                    Arc::new(settings_bg.clone()),
                    transcript_store_bg.clone(),
                    origin_turn_id_bg,
                )
                .await
            };

            let result = match AssertUnwindSafe(loop_future).catch_unwind().await {
                Ok(r) => r,
                Err(_) => Err(SubagentError {
                    message: "subagent panicked during execution".to_string(),
                    error_type: ErrorType::Unknown,
                    partial_result: None,
                }),
            };

            // Ensure the progress store always reflects a terminal status, even
            // if `run_subagent_loop` exited early via `?` (e.g. API timeout or
            // failure on the first round) without calling `emit`. The TUI
            // selector uses `is_terminal()` to decide when to remove a
            // subagent; without this, a non-terminal progress entry would keep
            // the subagent pinned in the selector forever.
            //
            // MERGE (not replace): only update `status` and `elapsed_ms`.
            // Preserve `cumulative_tokens`, `round`, `events`, and
            // `error_details` captured by the real terminal emit. If the real
            // emit's tokio::spawn write hasn't landed yet (race), fill in
            // `error_details` from the SubagentError result so diagnostics are
            // never NULL. This fixes the race where cleanup clobbered good
            // telemetry with zeros/defaults.
            {
                let (status, error_msg) = match &result {
                    Ok(_) => (SubagentStatus::Completed, None),
                    Err(e) => (SubagentStatus::Failed, Some(e.full_message())),
                };
                let now_ms = chrono::Utc::now().timestamp_millis();
                let mut store = progress_store_for_transcript.write().await;
                if let Some(m) = store.get_mut(&sid_bg) {
                    if let Some(p) = m.get_mut(&bg_node_id) {
                        p.status = status;
                        p.elapsed_ms = (now_ms - started_at_bg) as u64;
                        // Only fill error_details if the real emit didn't.
                        if p.error_details.is_none() {
                            if let Some(msg) = &error_msg {
                                p.error_details = Some(crate::agent::progress::ErrorInfo {
                                    error_type: ErrorType::Unknown,
                                    message: msg.clone(),
                                    last_tool: None,
                                    last_params: None,
                                    round: p.round.unwrap_or(0) as u32,
                                    retryable: false,
                                    ..Default::default()
                                });
                            }
                        }
                    }
                }
            }

            let (terminal, content, error_message) = match result {
                Ok(r) => (
                    ChildTerminal::Completed {
                        summary: r.chars().take(500).collect(),
                    },
                    r,
                    None,
                ),
                Err(e) => {
                    let msg = e.full_message();
                    (
                        ChildTerminal::Failed {
                            code: e.code().to_string(),
                            partial_result: None,
                        },
                        format!("Subagent error: {}", msg),
                        Some(msg),
                    )
                }
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

            // A report clears the route synchronously before the child reaches
            // this terminal path. Any other terminal outcome must fail closed:
            // record Escalate and consume the pending route rather than leaving
            // the root agent blocked forever or letting it edit around it.
            if let Some(runtime) = root_cause_runtime_bg {
                if let Err(error) = runtime.resolve_root_cause_child_terminal(
                    &root_cause_session_bg,
                    &root_cause_child_id_bg,
                ) {
                    tracing::error!(
                        child_id = %root_cause_child_id_bg,
                        error = %error,
                        "failed to resolve static root-cause child terminal"
                    );
                }
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
                // Pull real token/event/round telemetry from the last progress
                // callback the subagent loop emitted (defaults to 0/empty if
                // it never reported - e.g. early API failure).
                let (total_tokens, actual_rounds, events, failure_diagnostics) = {
                    let store = progress_store_for_transcript.read().await;
                    store
                        .get(&sid_bg)
                        .and_then(|m| m.get(&bg_node_id))
                        .map(|p| {
                            let events: Vec<crate::transcript::SubagentEventRecord> =
                                p.events.iter().map(convert_event).collect();
                            (
                                p.cumulative_tokens,
                                p.round.unwrap_or(0) as u32,
                                events,
                                p.error_details.clone(),
                            )
                        })
                        .unwrap_or((0, 0, Vec::new(), None))
                };
                let project_path = std::env::current_dir()
                    .ok()
                    .map(|p| p.to_string_lossy().to_string());
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
                    total_tokens,
                    actual_rounds,
                    token_budget,
                    error_message,
                    None, // summary
                    events,
                    failure_diagnostics,
                    project_path,
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
        if root_cause_route_bound {
            metadata.insert(
                "root_cause_route_bound".to_string(),
                serde_json::json!(true),
            );
        }
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

/// Mutating filesystem tools removed from explore/plan when `explore_readonly`.
const MUTATING_FS_TOOLS: &[&str] = &["file_write", "file_edit", "apply_patch"];

/// Map the model-supplied `subagent_type` string to a trusted [`NodeType`].
///
/// Mirrors the mapping in `cli/args.rs`. Unknown strings fall back to
/// `GeneralPurpose`, matching the pre-change `unwrap_or("general-purpose")`
/// behavior. The model JSON is never injected directly into `SpawnChildRequest`;
/// it always passes through this trusted dispatch-layer mapping.
fn parse_node_type(s: &str) -> crate::org_graph::NodeType {
    use crate::org_graph::NodeType;
    match s {
        "explore" => NodeType::Explore,
        "plan" => NodeType::Plan,
        "general-purpose" | "general" => NodeType::GeneralPurpose,
        "verify" | "verification" => NodeType::Verification,
        "root-cause" | "root_cause" => NodeType::RootCause,
        "guide" | "wgenty-code-guide" => NodeType::WgentyCodeGuide,
        _ => NodeType::GeneralPurpose,
    }
}

/// Filter the registry tool list for a node contract.
///
/// Three dimensions, all read from the same [`NodeContract`] the coordinator
/// enforces (the two-layer split from the spec):
/// - **capability** (`capabilities.allowed_tools`): when non-empty, a whitelist;
///   empty means wildcard (all pass). Builtin contracts use empty to mirror the
///   pre-change "all tools minus spawn/mutating-fs" semantics.
/// - **permission.can_spawn**: when false, `task`/`delegate` are stripped (leaf
///   nodes never coordinate).
/// - **permission.can_mutate_fs**: when false, [`MUTATING_FS_TOOLS`] are
///   stripped (driven by `explore_readonly` at contract-build time).
///
/// `exec_command` remains visible (`can_exec` is declared, not enforced here;
/// still gated by policy + guardian). Depth limiting is enforced by
/// `AgentCoordinator::reserve_child`.
pub(crate) fn filter_allowed_tools(
    names: impl IntoIterator<Item = String>,
    contract: &crate::org_graph::NodeContract,
) -> Vec<String> {
    names
        .into_iter()
        .filter(|name| {
            // Dimension 1: capability whitelist (empty = wildcard).
            if !contract.capabilities.allowed_tools.is_empty()
                && !contract.capabilities.allowed_tools.contains(name)
            {
                return false;
            }
            // Dimension 2: can_spawn — leaf nodes never get spawn tools.
            let is_spawn = name == "task" || name == "delegate";
            if is_spawn && !contract.permissions.can_spawn {
                return false;
            }
            // Dimension 3: can_mutate_fs — strip mutating FS tools.
            if MUTATING_FS_TOOLS.contains(&name.as_str()) && !contract.permissions.can_mutate_fs {
                return false;
            }
            true
        })
        .collect()
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
        CoordinatorError::ContractViolation {
            node_type,
            dimension,
            reason,
        } => ToolError {
            message: format!(
                "contract violation for {:?} node ({:?}): {}",
                node_type, dimension, reason
            ),
            code: Some("contract_violation".to_string()),
        },
        other => ToolError {
            message: other.to_string(),
            code: Some("coordinator_error".to_string()),
        },
    }
}
