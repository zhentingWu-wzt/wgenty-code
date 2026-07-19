//! Tools Module — the agent's hands: file I/O, search, command execution.
//!
//! Organized by capability domain:
//!   - filesystem/  — read, write, edit, apply_patch, list_files, view
//!   - search/      — grep, glob, full-text search
//!   - execution/   — shell commands, session management, git
//!   - meta/        — think, lsp, ask_user_question, note_edit

pub mod checkpoint;
pub mod checkpoint_store;
pub mod execution;
pub mod executor;
pub mod filesystem;
pub mod meta;
pub mod search;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use crate::agent::ToolContext;

/// Resolve a (possibly relative) file path against an optional per-agent workdir.
///
/// Absolute paths are returned unchanged. Relative paths join `workdir` when set
/// (s12 worktree isolation), otherwise fall back to the process cwd. The
/// returned path is owned so callers can use it directly in fs APIs.
pub fn resolve_path(file_path: &str, workdir: Option<&std::path::Path>) -> std::path::PathBuf {
    let p = std::path::Path::new(file_path);
    if p.is_absolute() {
        p.to_path_buf()
    } else if let Some(dir) = workdir {
        dir.join(p)
    } else {
        p.to_path_buf()
    }
}

/// Tool trait for all tools
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn input_schema(&self) -> serde_json::Value;

    fn is_read_only(&self) -> bool {
        false
    }

    /// Whether this tool requires explicit user confirmation before execution.
    /// External MCP tools use this when the client cannot prove they are
    /// read-only; built-in tools retain their existing specialized policies.
    fn requires_confirmation(&self) -> bool {
        false
    }

    async fn execute(&self, input: serde_json::Value) -> Result<ToolOutput, ToolError>;

    /// Contextual execution path.
    ///
    /// Identity-sensitive tools override this to read the trusted
    /// [`ToolContext`] (agent identity, session, depth, cancellation) instead
    /// of trusting model-supplied JSON. The default adapter delegates to
    /// [`execute`](Self::execute) so context-free tools need no changes; they
    /// simply ignore the context.
    async fn execute_with_context(
        &self,
        _context: &ToolContext<'_>,
        input: serde_json::Value,
    ) -> Result<ToolOutput, ToolError> {
        self.execute(input).await
    }

    fn tool_definition(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": self.name(),
                "description": self.description(),
                "parameters": self.input_schema()
            }
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolOutput {
    pub output_type: String,
    pub content: String,
    pub metadata: HashMap<String, serde_json::Value>,
}

impl ToolOutput {
    /// Creates a plain-text tool output with no metadata.
    pub fn text(content: impl Into<String>) -> Self {
        Self {
            output_type: "text".to_string(),
            content: content.into(),
            metadata: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolError {
    pub message: String,
    pub code: Option<String>,
}

pub struct ToolRegistry {
    tools: RwLock<HashMap<String, Arc<dyn Tool>>>,
    pub checkpoint_manager: Arc<CheckpointManager>,
    pub checkpoint_store: Arc<CheckpointStore>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self::with_project_root(
            std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")),
            checkpoint_store::DEFAULT_KEEP_N,
        )
    }

    /// Construct a registry scoped to `project_root` (checkpoint snapshots live
    /// under `<project_root>/.wgenty-code/checkpoints/`).
    pub fn with_project_root(project_root: impl Into<std::path::PathBuf>, keep_n: usize) -> Self {
        let sandbox = std::sync::Arc::new(crate::sandbox::SandboxManager::new());
        let command_sessions = std::sync::Arc::new(
            execution::session_manager::CommandSessionManager::new().with_sandbox(sandbox.clone()),
        );
        let store = std::sync::Arc::new(CheckpointStore::with_keep_n(project_root, keep_n));
        let checkpoint_manager =
            std::sync::Arc::new(checkpoint::CheckpointManager::new(store.clone()));
        let registry = Self {
            tools: RwLock::new(HashMap::new()),
            checkpoint_manager: checkpoint_manager.clone(),
            checkpoint_store: store,
        };

        // Checkpoint tools
        registry.register(Box::new(checkpoint::CheckpointTool::new(
            checkpoint_manager.clone(),
        )));
        registry.register(Box::new(checkpoint::UndoTool::new(
            checkpoint_manager.clone(),
        )));
        // Meta tools
        registry.register(Box::new(meta::ask_user_question::AskUserQuestionTool::new()));
        registry.register(Box::new(meta::update_plan::UpdatePlanTool::new()));
        registry.register(Box::new(meta::skill::SkillTool::new()));
        registry.register(Box::new(meta::team_message::TeamMessageTool::new()));
        registry.register(Box::new(meta::request_approval::RequestApprovalTool::new()));
        // Filesystem tools
        registry.register(Box::new(filesystem::apply_patch::ApplyPatchTool::new()));
        registry.register(Box::new(filesystem::file_read::FileReadTool::new()));
        registry.register(Box::new(filesystem::file_edit::FileEditTool::new()));
        registry.register(Box::new(filesystem::file_write::FileWriteTool::new()));
        // Execution tools
        registry.register(Box::new(
            execution::execute_command::ExecuteCommandTool::with_sandbox(sandbox.clone()),
        ));
        registry.register(Box::new(execution::exec_command::ExecCommandTool::new(
            command_sessions.clone(),
        )));
        registry.register(Box::new(execution::write_stdin::WriteStdinTool::new(
            command_sessions.clone(),
        )));
        registry.register(Box::new(execution::kill_session::KillSessionTool::new(
            command_sessions,
        )));
        // Search tools
        registry.register(Box::new(search::search::SearchTool::new()));
        registry.register(Box::new(search::web_search::WebSearchTool::new()));
        registry.register(Box::new(search::web_fetch::WebFetchTool::new()));
        registry.register(Box::new(search::glob_search::GlobTool::new()));
        registry.register(Box::new(search::grep::GrepTool::new()));
        // Filesystem tools (more)
        registry.register(Box::new(filesystem::list_files::ListFilesTool::new()));
        registry.register(Box::new(filesystem::view::ViewTool::new()));
        // Execution tools (git)
        registry.register(Box::new(execution::git_operations::GitOperationsTool::new()));
        // Execution tools (test runner)
        registry.register(Box::new(execution::run_test::RunTestTool::new(sandbox)));
        // Meta tools (more)
        registry.register(Box::new(meta::think::ThinkTool::new()));
        registry.register(Box::new(meta::compact::CompactTool::new()));
        registry.register(Box::new(meta::lsp::LspTool::new()));
        registry.register(Box::new(meta::note_edit::NoteEditTool::new()));
        registry.register(Box::new(
            meta::dismiss_codegraph_guidance::DismissCodegraphGuidanceTool::new(),
        ));
        registry.register(Box::new(crate::tasks::management::TaskManagementTool::new()));
        registry
    }

    /// Register the ExecutionSession `verify_and_complete` tool bound to
    /// `coordinator` (Task 7). Frontends call this after constructing the
    /// session coordinator so the agent can call `verify_and_complete` to mark
    /// a session `Completed`. The tool shares the same
    /// `Arc<RwLock<SessionCoordinator>>` as the agent-loop turn hook, so
    /// turn-boundary bookkeeping and verify-gate transitions act on one
    /// session. Uses default hooks (`AutoRetry { max: 2 }` on verify failure)
    /// and a `ProcessCommandExecutor` (production can swap an executor that
    /// routes through guardian + sandbox).
    pub fn register_exec_session_tools(
        &self,
        coordinator: Arc<RwLock<crate::exec_session::SessionCoordinator>>,
    ) {
        let gate = Arc::new(crate::exec_session::VerifyGate::new_with_default_hooks(
            coordinator,
            Arc::new(crate::exec_session::ProcessCommandExecutor),
        ));
        self.register(Box::new(crate::exec_session::VerifyAndCompleteTool::new(
            gate,
        )));
    }

    /// Apply provider-aware configuration after construction.
    ///
    /// Nearly all major providers now ship with built-in web search:
    /// Anthropic (web_search_20250305), OpenAI, 百度/文心, 千问/通义,
    /// Kimi/月之暗面, 豆包, 腾讯元宝, Gemini, etc.
    ///
    /// Only register a local web_search tool for providers that explicitly
    /// lack native search capability (DeepSeek, self-hosted Ollama/vLLM).
    /// The local tool uses DuckDuckGo by default (zero-config), with optional
    /// Tavily fallback.
    pub fn with_settings(self, settings: &crate::config::Settings) -> Self {
        let provider = crate::api::provider::resolve_provider(
            &settings.models.main.endpoint_base_url(),
            settings.models.main.provider.as_deref(),
        );

        // Whitelist: only these providers lack built-in web search.
        const PROVIDERS_WITHOUT_BUILTIN_SEARCH: &[&str] = &["deepseek", "openai"];

        // Note: "openai" here refers to the catch-all OpenAI-compatible path
        // (Ollama, vLLM, local models, etc.) — the default fallback provider.
        // The "openai" provider maps to unknown/self-hosted endpoints that
        // typically don't have built-in search.

        if !PROVIDERS_WITHOUT_BUILTIN_SEARCH.contains(&provider.name()) {
            self.tools
                .write()
                .expect("lock poisoned: tools")
                .remove("web_search");
            tracing::info!(
                "web_search tool skipped: {} has built-in search capability",
                provider.name()
            );
        }

        self
    }

    /// Register a tool. Takes `&self` so it works on an `Arc<ToolRegistry>`,
    /// enabling runtime registration (e.g. background MCP tool connection).
    pub fn register(&self, tool: Box<dyn Tool>) {
        let name = tool.name().to_string();
        self.tools
            .write()
            .expect("lock poisoned: tools")
            .insert(name, Arc::from(tool));
    }

    /// Register a remote tool, preserving its standard name when available and
    /// prefixing it with the server name only when it collides with an existing
    /// built-in or remote tool.
    pub fn register_external(&self, server_name: &str, tool: Box<dyn Tool>) -> String {
        let original_name = tool.name().to_string();
        let mut tools = self.tools.write().expect("lock poisoned: tools");
        if !tools.contains_key(&original_name) {
            tools.insert(original_name.clone(), Arc::from(tool));
            return original_name;
        }

        let exposed_name = format!("{server_name}__{original_name}");
        tools.insert(exposed_name.clone(), Arc::from(tool));
        exposed_name
    }

    /// Wire the external skill registry into the `skill` tool so it can resolve external skills.
    ///
    /// Replaces the existing `SkillTool` (created via `SkillTool::new()` without a registry)
    /// with one that has the registry wired, enabling the model to invoke external skills
    /// via the `skill` tool.
    pub fn wire_skill_registry(&self, registry: Arc<crate::knowledge::ExternalSkillRegistry>) {
        let new_tool: Box<dyn Tool> = Box::new(meta::skill::SkillTool::with_registry(
            registry,
            crate::knowledge::LoadedSkillContext::default(),
        ));
        self.tools
            .write()
            .expect("lock poisoned: tools")
            .insert("skill".to_string(), Arc::from(new_tool));
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools
            .read()
            .expect("lock poisoned: tools")
            .get(name)
            .cloned()
    }

    pub fn list(&self) -> Vec<Arc<dyn Tool>> {
        self.tools
            .read()
            .expect("lock poisoned: tools")
            .values()
            .cloned()
            .collect()
    }

    pub async fn execute(
        &self,
        name: &str,
        input: serde_json::Value,
    ) -> Result<ToolOutput, ToolError> {
        let tool = self
            .tools
            .read()
            .expect("lock poisoned: tools")
            .get(name)
            .cloned();
        match tool {
            Some(tool) => tool.execute(input).await,
            None => Err(ToolError {
                message: format!("Tool not found: {}", name),
                code: Some("tool_not_found".to_string()),
            }),
        }
    }

    /// Executes a tool with the trusted [`ToolContext`].
    ///
    /// This is the path identity-sensitive tools must use: the agent identity,
    /// session, depth, and cancellation token come from trusted runtime state,
    /// never from model-supplied JSON. Forging `_agent_id`/`_session_id`/
    /// `_subagent_depth` in `input` has no effect because the context is
    /// authoritative.
    ///
    /// Before executing mutating filesystem tools (`file_edit`, `file_write`,
    /// `apply_patch`), captures pre-edit file content into the active turn
    /// snapshot when `context.checkpoint` and `context.origin_turn_id` are set.
    pub async fn execute_with_context(
        &self,
        context: &ToolContext<'_>,
        name: &str,
        input: serde_json::Value,
    ) -> Result<ToolOutput, ToolError> {
        maybe_capture_pre_edit(context, name, &input);
        let tool = self
            .tools
            .read()
            .expect("lock poisoned: tools")
            .get(name)
            .cloned();
        match tool {
            Some(tool) => tool.execute_with_context(context, input).await,
            None => Err(ToolError {
                message: format!("Tool not found: {}", name),
                code: Some("tool_not_found".to_string()),
            }),
        }
    }
}

/// Best-effort pre-edit capture for mutating filesystem tools. Never blocks
/// the tool call: missing context / path extraction failures are ignored
/// (logged by the capture implementation).
fn maybe_capture_pre_edit(context: &ToolContext<'_>, tool_name: &str, args: &serde_json::Value) {
    let (Some(capture), Some(turn_id)) = (context.checkpoint, context.origin_turn_id) else {
        return;
    };
    if context.effective_mode == crate::sandbox::EffectiveMode::Plan {
        return;
    }
    let paths = match tool_name {
        "file_edit" | "file_write" => args
            .get("path")
            .and_then(|v| v.as_str())
            .map(|p| vec![p.to_string()])
            .unwrap_or_default(),
        "apply_patch" => args
            .get("patch")
            .and_then(|v| v.as_str())
            .map(filesystem::extract_patch_paths)
            .unwrap_or_default(),
        _ => return,
    };
    for path in paths {
        if path.is_empty() {
            continue;
        }
        let abs = resolve_path(&path, context.workdir);
        capture.capture_file(turn_id, &abs);
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// Re-export all tool types
pub use checkpoint::{CheckpointManager, CheckpointTool, UndoTool};
pub use checkpoint_store::CheckpointStore;
pub use execution::{
    BackgroundManager, BackgroundResult, BackgroundTool, CommandSessionManager, ExecCommandTool,
    ExecuteCommandTool, GitOperationsTool, KillSessionTool, RunTestTool, WriteStdinTool,
};
pub use executor::ToolExecutor;
pub use filesystem::{
    ApplyPatchTool, FileEditTool, FileReadTool, FileWriteTool, ListFilesTool, ViewTool,
};
pub use meta::{
    AskUserQuestionTool, CompactTool, LoadSkillTool, LspTool, NoteEditTool, RlmDelegateTool,
    SkillTool, SubagentTraceTool, TaskTool, TeamMessageTool, ThinkTool, UpdatePlanTool,
};
pub use search::{GlobTool, GrepTool, SearchTool, WebFetchTool, WebSearchTool};

#[cfg(test)]
mod external_tool_tests {
    use super::*;

    struct NamedTool(&'static str);

    #[async_trait]
    impl Tool for NamedTool {
        fn name(&self) -> &str {
            self.0
        }

        fn description(&self) -> &str {
            "test"
        }

        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object"})
        }

        async fn execute(&self, _input: serde_json::Value) -> Result<ToolOutput, ToolError> {
            Ok(ToolOutput {
                output_type: "text".to_string(),
                content: String::new(),
                metadata: HashMap::new(),
            })
        }
    }

    /// A probe tool that records the trusted caller identity from the
    /// contextual path and panics if the context-free path is used.
    struct ContextProbe;

    #[async_trait]
    impl Tool for ContextProbe {
        fn name(&self) -> &str {
            "context_probe"
        }
        fn description(&self) -> &str {
            "records trusted context"
        }
        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object"})
        }
        async fn execute(&self, _input: serde_json::Value) -> Result<ToolOutput, ToolError> {
            panic!("contextual path must be used")
        }
        async fn execute_with_context(
            &self,
            context: &ToolContext<'_>,
            _input: serde_json::Value,
        ) -> Result<ToolOutput, ToolError> {
            Ok(ToolOutput::text(context.agent.agent_id.to_string()))
        }
    }

    #[tokio::test]
    async fn execute_with_context_uses_trusted_agent_id_not_forged_input() {
        let registry = ToolRegistry::new();
        registry.register(Box::new(ContextProbe));

        let root = crate::agent::AgentExecutionContext::root(crate::agent::SessionId::new("s"));
        let invocation_id = crate::agent::ToolInvocationId::new("inv-1");
        let context = ToolContext {
            agent: &root,
            invocation_id,
            origin_turn_id: None,
            workdir: None,
            effective_mode: crate::sandbox::EffectiveMode::default(),
            checkpoint: None,
        };

        // Input carries forged identity fields; they must be ignored.
        let forged = serde_json::json!({
            "_agent_id": "forged-agent",
            "_session_id": "forged-session",
            "_subagent_depth": 0,
        });

        let output = registry
            .execute_with_context(&context, "context_probe", forged)
            .await
            .unwrap();

        assert_eq!(output.content, root.agent_id.to_string());
        assert_ne!(output.content, "forged-agent");
    }

    #[tokio::test]
    async fn execute_with_context_defaults_to_context_free_execute() {
        // NamedTool does not override execute_with_context, so the default
        // adapter must delegate to execute.
        let registry = ToolRegistry::new();
        registry.register(Box::new(NamedTool("adapter_probe")));

        let root = crate::agent::AgentExecutionContext::root(crate::agent::SessionId::new("s"));
        let context = ToolContext {
            agent: &root,
            invocation_id: crate::agent::ToolInvocationId::new("inv-2"),
            origin_turn_id: None,
            workdir: None,
            effective_mode: crate::sandbox::EffectiveMode::default(),
            checkpoint: None,
        };

        let output = registry
            .execute_with_context(&context, "adapter_probe", serde_json::json!({}))
            .await
            .unwrap();
        assert_eq!(output.output_type, "text");
    }

    #[tokio::test]
    async fn pre_edit_capture_runs_before_file_write_and_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::write(root.join("a.txt"), "original").unwrap();

        let registry = ToolRegistry::with_project_root(root, 10);
        let store = registry.checkpoint_store.clone();
        store.begin_turn("turn-1").unwrap();

        let root_agent =
            crate::agent::AgentExecutionContext::root(crate::agent::SessionId::new("s"));
        let context = ToolContext {
            agent: &root_agent,
            invocation_id: crate::agent::ToolInvocationId::new("inv-cap"),
            origin_turn_id: Some("turn-1"),
            workdir: Some(root),
            effective_mode: crate::sandbox::EffectiveMode::default(),
            checkpoint: Some(store.as_ref()),
        };

        // First write captures pre-edit content then overwrites.
        registry
            .execute_with_context(
                &context,
                "file_write",
                serde_json::json!({"path": "a.txt", "content": "first"}),
            )
            .await
            .unwrap();
        // Second write of same file must not replace the first capture.
        registry
            .execute_with_context(
                &context,
                "file_write",
                serde_json::json!({"path": "a.txt", "content": "second"}),
            )
            .await
            .unwrap();

        assert_eq!(
            std::fs::read_to_string(root.join("a.txt")).unwrap(),
            "second"
        );
        let summary = store.rewind("turn-1").unwrap();
        assert!(summary.contains("restored 1"), "{summary}");
        assert_eq!(
            std::fs::read_to_string(root.join("a.txt")).unwrap(),
            "original"
        );
    }

    #[tokio::test]
    async fn pre_edit_capture_skipped_in_plan_mode() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::write(root.join("a.txt"), "original").unwrap();
        let registry = ToolRegistry::with_project_root(root, 10);
        let store = registry.checkpoint_store.clone();
        store.begin_turn("turn-plan").unwrap();
        let root_agent =
            crate::agent::AgentExecutionContext::root(crate::agent::SessionId::new("s"));
        let context = ToolContext {
            agent: &root_agent,
            invocation_id: crate::agent::ToolInvocationId::new("inv-plan"),
            origin_turn_id: Some("turn-plan"),
            workdir: Some(root),
            effective_mode: crate::sandbox::EffectiveMode::Plan,
            checkpoint: Some(store.as_ref()),
        };
        registry
            .execute_with_context(
                &context,
                "file_write",
                serde_json::json!({"path": "a.txt", "content": "changed"}),
            )
            .await
            .unwrap();
        let infos = store.list().unwrap();
        let turn = infos.iter().find(|t| t.turn_id == "turn-plan").unwrap();
        assert_eq!(turn.file_count, 0, "plan mode must not capture files");
    }

    #[test]
    fn external_tools_keep_names_unless_they_collide() {
        let registry = ToolRegistry::new();
        let first = registry.register_external("codegraph", Box::new(NamedTool("remote_unique")));
        let second = registry.register_external("other", Box::new(NamedTool("remote_unique")));

        assert_eq!(first, "remote_unique");
        assert_eq!(second, "other__remote_unique");
        assert!(registry.get("remote_unique").is_some());
        assert!(registry.get("other__remote_unique").is_some());
    }
}
