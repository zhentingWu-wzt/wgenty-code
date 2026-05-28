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

use crate::api::ApiClient;
use crate::config::Settings;
use crate::teams::subagent_loop::run_subagent_loop;
use crate::tools::{Tool, ToolError, ToolOutput};
use async_trait::async_trait;
use std::collections::HashMap;

pub struct TaskTool {
    settings: Settings,
    tool_registry: std::sync::Weak<crate::tools::ToolRegistry>,
}

impl TaskTool {
    pub fn new(
        settings: Settings,
        tool_registry: std::sync::Weak<crate::tools::ToolRegistry>,
    ) -> Self {
        Self {
            settings,
            tool_registry,
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
                "prompt": {
                    "type": "string",
                    "description": "The detailed task for the subagent to perform"
                }
            },
            "required": ["description", "prompt"]
        })
    }

    async fn execute(&self, input: serde_json::Value) -> Result<ToolOutput, ToolError> {
        let subagent_type = input["subagent_type"].as_str().unwrap_or("general-purpose");
        let description = input["description"].as_str().unwrap_or("Subagent task");
        let prompt = input["prompt"].as_str().unwrap_or("");

        // Upgrade the Weak reference to the tool registry.
        let tool_registry = self.tool_registry.upgrade().ok_or_else(|| ToolError {
            message: "Tool registry is no longer available".to_string(),
            code: Some("registry_dropped".to_string()),
        })?;

        // Filter tools: exclude "task" to prevent recursive subagent spawning.
        let allowed_tools: Vec<String> = tool_registry
            .list()
            .iter()
            .map(|t| t.name().to_string())
            .filter(|name| name != "task")
            .collect();

        // Build system prompt based on subagent type.
        let system_prompt = match subagent_type {
            "explore" => {
                "You are a code exploration subagent. Your role is to search and \
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
                "You are a planning subagent. Your role is to break down complex \
                 tasks into actionable steps.\n\nKey responsibilities:\n\
                 1. Analyze task requirements\n\
                 2. Identify key files and components\n\
                 3. Break down the work into logical steps\n\
                 4. Consider dependencies, risks, and trade-offs\n\n\
                 Use file_read and search tools to understand the codebase before \
                 planning. Be thorough and structured in your analysis."
            }
            _ => {
                "You are a general-purpose subagent. Complete the assigned task \
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

        // Create a fresh API client from the stored settings.
        let api_client = ApiClient::new(self.settings.clone());

        // Run the subagent loop (capped at 10 rounds).
        match run_subagent_loop(
            &api_client,
            &tool_registry,
            system_prompt,
            &full_prompt,
            &allowed_tools,
            10,
        )
        .await
        {
            Ok(result) => {
                let mut metadata = HashMap::new();
                metadata.insert(
                    "subagent_type".to_string(),
                    serde_json::json!(subagent_type),
                );
                metadata.insert("description".to_string(), serde_json::json!(description));

                Ok(ToolOutput {
                    output_type: "text".to_string(),
                    content: result,
                    metadata,
                })
            }
            Err(e) => Err(ToolError {
                message: e,
                code: Some("subagent_error".to_string()),
            }),
        }
    }
}
