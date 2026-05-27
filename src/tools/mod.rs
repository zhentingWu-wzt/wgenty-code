//! Tools Module - File operations, commands, search, etc.

pub mod ask_user_question;
pub mod apply_patch;
pub mod view;
pub mod executor;
pub mod execute_command;
pub mod exec_command;
pub mod file_edit;
pub mod file_read;
pub mod file_write;
pub mod git_operations;
pub mod glob_search;
pub mod grep;
pub mod kill_session;
pub mod list_files;
pub mod lsp;
pub mod note_edit;
pub mod policy;
pub mod search;
pub mod session_manager;
pub mod task_management;
pub mod think;
pub mod write_stdin;

pub use ask_user_question::AskUserQuestionTool;
pub use apply_patch::ApplyPatchTool;
pub use view::ViewTool;
pub use executor::ToolExecutor;
pub use execute_command::ExecuteCommandTool;
pub use exec_command::ExecCommandTool;
pub use file_edit::FileEditTool;
pub use file_read::FileReadTool;
pub use file_write::FileWriteTool;
pub use git_operations::GitOperationsTool;
pub use glob_search::GlobTool;
pub use grep::GrepTool;
pub use kill_session::KillSessionTool;
pub use list_files::ListFilesTool;
pub use lsp::LspTool;
pub use note_edit::NoteEditTool;
pub use policy::{PermissionRequest, PolicyDecision, ToolPermissionPolicy};
pub use search::SearchTool;
pub use session_manager::CommandSessionManager;
pub use task_management::TaskManagementTool;
pub use think::ThinkTool;
pub use write_stdin::WriteStdinTool;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Tool trait for all tools
#[async_trait]
pub trait Tool: Send + Sync {
    /// Tool name
    fn name(&self) -> &str;

    /// Tool description
    fn description(&self) -> &str;

    /// Tool input schema
    fn input_schema(&self) -> serde_json::Value;

    /// Whether this tool is read-only (no side effects). Default: false.
    fn is_read_only(&self) -> bool {
        false
    }

    /// Execute the tool
    async fn execute(&self, input: serde_json::Value) -> Result<ToolOutput, ToolError>;

    /// Convert to OpenAI-compatible function definition
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

/// Tool output
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolOutput {
    /// Output type
    pub output_type: String,
    /// Output content
    pub content: String,
    /// Metadata
    pub metadata: HashMap<String, serde_json::Value>,
}

/// Tool error
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolError {
    /// Error message
    pub message: String,
    /// Error code
    pub code: Option<String>,
}

/// Tool registry
pub struct ToolRegistry {
    /// Registered tools
    tools: HashMap<String, Box<dyn Tool>>,
}

impl ToolRegistry {
    /// Create a new tool registry
    pub fn new() -> Self {
        let command_sessions = std::sync::Arc::new(session_manager::CommandSessionManager::new());
        let mut registry = Self {
            tools: HashMap::new(),
        };

        // Register built-in tools
        registry.register(Box::new(ask_user_question::AskUserQuestionTool::new()));
        registry.register(Box::new(apply_patch::ApplyPatchTool::new()));
        registry.register(Box::new(file_read::FileReadTool::new()));
        registry.register(Box::new(file_edit::FileEditTool::new()));
        registry.register(Box::new(file_write::FileWriteTool::new()));
        registry.register(Box::new(execute_command::ExecuteCommandTool::new()));
        registry.register(Box::new(exec_command::ExecCommandTool::new(
            command_sessions.clone(),
        )));
        registry.register(Box::new(write_stdin::WriteStdinTool::new(
            command_sessions.clone(),
        )));
        registry.register(Box::new(kill_session::KillSessionTool::new(
            command_sessions,
        )));
        registry.register(Box::new(search::SearchTool::new()));
        registry.register(Box::new(glob_search::GlobTool::new()));
        registry.register(Box::new(grep::GrepTool::new()));
        registry.register(Box::new(list_files::ListFilesTool::new()));
        registry.register(Box::new(view::ViewTool::new()));
        registry.register(Box::new(git_operations::GitOperationsTool::new()));
        registry.register(Box::new(task_management::TaskManagementTool::new()));
        registry.register(Box::new(think::ThinkTool::new()));
        registry.register(Box::new(lsp::LspTool::new()));
        registry.register(Box::new(note_edit::NoteEditTool::new()));

        registry
    }

    /// Register a tool
    pub fn register(&mut self, tool: Box<dyn Tool>) {
        self.tools.insert(tool.name().to_string(), tool);
    }

    /// Get a tool by name
    pub fn get(&self, name: &str) -> Option<&dyn Tool> {
        self.tools.get(name).map(|b| b.as_ref())
    }

    /// List all tools
    pub fn list(&self) -> Vec<&dyn Tool> {
        self.tools.values().map(|b| b.as_ref()).collect()
    }

    /// Execute a tool
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
