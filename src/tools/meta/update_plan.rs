//! Update Plan Tool
//!
//! A tool that updates the structured plan displayed in the UI panel.
//! The actual interactive logic is handled specially in the TUI layer,
//! but this tool defines the schema so the LLM knows when and how to invoke it.

use crate::tools::{Tool, ToolError, ToolOutput};
use async_trait::async_trait;

pub struct UpdatePlanTool;

impl Default for UpdatePlanTool {
    fn default() -> Self {
        Self::new()
    }
}

impl UpdatePlanTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for UpdatePlanTool {
    fn name(&self) -> &str {
        "update_plan"
    }

    fn description(&self) -> &str {
        "Present or update the structured plan shown in the UI panel. Each call replaces the entire plan. Set status to: pending, in_progress, or completed. Use at the start of a task, and call again to reflect progress."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "plan": {
                    "type": "array",
                    "description": "The full list of plan steps. Each call replaces the plan entirely.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "step": {
                                "type": "string",
                                "description": "A concrete, actionable step description"
                            },
                            "status": {
                                "type": "string",
                                "enum": ["pending", "in_progress", "completed"],
                                "description": "Current status of this step"
                            }
                        },
                        "required": ["step", "status"]
                    }
                }
            },
            "required": ["plan"]
        })
    }

    async fn execute(&self, _input: serde_json::Value) -> Result<ToolOutput, ToolError> {
        Err(ToolError {
            message: "update_plan requires interactive execution in the TUI".to_string(),
            code: Some("interactive_required".to_string()),
        })
    }
}
