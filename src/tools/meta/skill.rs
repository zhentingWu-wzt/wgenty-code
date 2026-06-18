//! Skill Runtime Action — Claude Code-compatible `skill` tool for nested external skill loading.
//!
//! When the external skill registry is not wired this tool returns a clear
//! not-configured error, signalling that the runtime needs to be set up before
//! skill resolution is available.

use crate::tools::{Tool, ToolError, ToolOutput};
use async_trait::async_trait;

/// Claude Code-compatible runtime action for loading external skills.
pub struct SkillTool;

impl SkillTool {
    /// Create a new skill tool instance.
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for SkillTool {
    fn name(&self) -> &str {
        "skill"
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn description(&self) -> &str {
        "Load a Claude Code-compatible external skill by canonical name. \
         Use for nested skill invocation."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "skill": {
                    "type": "string",
                    "description": "Canonical external skill name to load"
                },
                "args": {
                    "type": "string",
                    "description": "Optional raw arguments passed to the skill"
                },
                "depth": {
                    "type": "integer",
                    "description": "Nested skill depth"
                }
            },
            "required": ["skill"]
        })
    }

    async fn execute(&self, _input: serde_json::Value) -> Result<ToolOutput, ToolError> {
        Err(ToolError {
            message: "External skill registry is not configured".to_string(),
            code: Some("skill_registry_unconfigured".to_string()),
        })
    }
}
