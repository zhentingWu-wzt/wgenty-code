//! Tools Module — the agent's hands: file I/O, search, command execution.
//!
//! Organized by capability domain:
//!   - filesystem/  — read, write, edit, apply_patch, list_files, view
//!   - search/      — grep, glob, full-text search
//!   - execution/   — shell commands, session management, git
//!   - meta/        — think, lsp, ask_user_question, note_edit

pub mod checkpoint;
pub mod codegraph;
pub mod execution;
pub mod executor;
pub mod filesystem;
pub mod meta;
pub mod search;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

/// Tool trait for all tools
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn input_schema(&self) -> serde_json::Value;

    fn is_read_only(&self) -> bool {
        false
    }

    async fn execute(&self, input: serde_json::Value) -> Result<ToolOutput, ToolError>;

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolError {
    pub message: String,
    pub code: Option<String>,
}

pub struct ToolRegistry {
    tools: HashMap<String, Box<dyn Tool>>,
    pub checkpoint_manager: Arc<CheckpointManager>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        let sandbox = std::sync::Arc::new(crate::sandbox::SandboxManager::new());
        let command_sessions = std::sync::Arc::new(
            execution::session_manager::CommandSessionManager::new().with_sandbox(sandbox.clone()),
        );
        let checkpoint_manager = std::sync::Arc::new(checkpoint::CheckpointManager::new());
        let mut registry = Self {
            tools: HashMap::new(),
            checkpoint_manager: checkpoint_manager.clone(),
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
        registry.register(Box::new(crate::tasks::management::TaskManagementTool::new()));
        // CodeGraph tools (lazy-init from .codegraph/index.db)
        registry.register(Box::new(codegraph::tools::CodegraphNodeTool::new()));
        registry.register(Box::new(codegraph::tools::CodegraphExploreTool::new()));
        registry.register(Box::new(codegraph::tools::CallPathTool::new()));
        registry.register(Box::new(codegraph::tools::SymbolBatchTool::new()));
        registry.register(Box::new(codegraph::tools::ModuleSummaryTool::new()));

        registry
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
    pub fn with_settings(mut self, settings: &crate::config::Settings) -> Self {
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
            self.tools.remove("web_search");
            tracing::info!(
                "web_search tool skipped: {} has built-in search capability",
                provider.name()
            );
        }

        self
    }

    pub fn register(&mut self, tool: Box<dyn Tool>) {
        self.tools.insert(tool.name().to_string(), tool);
    }

    /// Wire the external skill registry into the `skill` tool so it can resolve external skills.
    ///
    /// Replaces the existing `SkillTool` (created via `SkillTool::new()` without a registry)
    /// with one that has the registry wired, enabling the model to invoke external skills
    /// via the `skill` tool.
    pub fn wire_skill_registry(&mut self, registry: Arc<crate::knowledge::ExternalSkillRegistry>) {
        let new_tool = Box::new(meta::skill::SkillTool::with_registry(
            registry,
            crate::knowledge::LoadedSkillContext::default(),
        ));
        self.tools.insert("skill".to_string(), new_tool);
    }

    pub fn get(&self, name: &str) -> Option<&dyn Tool> {
        self.tools.get(name).map(|b| b.as_ref())
    }

    pub fn list(&self) -> Vec<&dyn Tool> {
        self.tools.values().map(|b| b.as_ref()).collect()
    }

    pub async fn execute(
        &self,
        name: &str,
        input: serde_json::Value,
    ) -> Result<ToolOutput, ToolError> {
        match self.tools.get(name) {
            Some(tool) => tool.execute(input).await,
            None => Err(ToolError {
                message: format!("Tool not found: {}", name),
                code: Some("tool_not_found".to_string()),
            }),
        }
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// Re-export all tool types
pub use checkpoint::{CheckpointManager, CheckpointTool, UndoTool};
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
