//! Tools Module — the agent's hands: file I/O, search, command execution.
//!
//! Organized by capability domain:
//!   - filesystem/  — read, write, edit, apply_patch, list_files, view
//!   - search/      — grep, glob, full-text search
//!   - execution/   — shell commands, session management, git
//!   - meta/        — think, lsp, ask_user_question, note_edit

pub mod execution;
pub mod executor;
pub mod filesystem;
pub mod meta;
pub mod search;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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
}

impl ToolRegistry {
    pub fn new() -> Self {
        // Create sandbox manager (auto-selects best backend)
        let sandbox = std::sync::Arc::new(crate::sandbox::SandboxManager::new());
        let command_sessions = std::sync::Arc::new(
            execution::session_manager::CommandSessionManager::new()
                .with_sandbox(sandbox.clone()),
        );
        let mut registry = Self {
            tools: HashMap::new(),
        };

        // Meta tools
        registry.register(Box::new(meta::ask_user_question::AskUserQuestionTool::new()));
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
        registry.register(Box::new(search::glob_search::GlobTool::new()));
        registry.register(Box::new(search::grep::GrepTool::new()));
        // Filesystem tools (more)
        registry.register(Box::new(filesystem::list_files::ListFilesTool::new()));
        registry.register(Box::new(filesystem::view::ViewTool::new()));
        // Execution tools (git)
        registry.register(Box::new(execution::git_operations::GitOperationsTool::new()));
        // Meta tools (more)
        registry.register(Box::new(meta::think::ThinkTool::new()));
        registry.register(Box::new(meta::compact::CompactTool::new()));
        registry.register(Box::new(meta::lsp::LspTool::new()));
        registry.register(Box::new(meta::note_edit::NoteEditTool::new()));
        // Task management is registered externally via DaemonState to share task store

        registry
    }

    pub fn register(&mut self, tool: Box<dyn Tool>) {
        self.tools.insert(tool.name().to_string(), tool);
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
pub use execution::{
    BackgroundManager, BackgroundResult, BackgroundTool, CommandSessionManager, ExecCommandTool,
    ExecuteCommandTool, GitOperationsTool, KillSessionTool, WriteStdinTool,
};
pub use executor::ToolExecutor;
pub use filesystem::{
    ApplyPatchTool, FileEditTool, FileReadTool, FileWriteTool, ListFilesTool, ViewTool,
};
pub use meta::{
    AskUserQuestionTool, CompactTool, LoadSkillTool, LspTool, NoteEditTool, TaskTool,
    TeamMessageTool, ThinkTool,
};
pub use search::{GlobTool, GrepTool, SearchTool};
