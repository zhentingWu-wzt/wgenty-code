//! Ask User Question Tool
//!
//! A tool that pauses the agent loop to ask the user a clarifying question
//! with structured choices. The actual interactive logic is handled specially
//! in the REPL layer, but this tool defines the schema so the LLM knows when
//! and how to invoke it.

use super::{Tool, ToolError, ToolOutput};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

pub struct AskUserQuestionTool;

impl Default for AskUserQuestionTool {
    fn default() -> Self {
        Self::new()
    }
}

impl AskUserQuestionTool {
    pub fn new() -> Self {
        Self
    }
}

/// A single option for the user to choose from.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuestionOption {
    /// Short display label (1-5 words)
    pub label: String,
    /// Detailed explanation of this option
    pub description: String,
    /// Optional preview content (markdown) shown when focused
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
}

#[async_trait]
impl Tool for AskUserQuestionTool {
    fn name(&self) -> &str {
        "ask_user_question"
    }

    fn description(&self) -> &str {
        "Ask the user a clarifying question with structured choices. Use this when the request is ambiguous, incomplete, or you need more information before proceeding."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "question": {
                    "type": "string",
                    "description": "The question to ask the user"
                },
                "options": {
                    "type": "array",
                    "description": "Available answer options. Always include 2-4 options plus an implicit 'Other'.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "label": {
                                "type": "string",
                                "description": "Short option label (1-5 words)"
                            },
                            "description": {
                                "type": "string",
                                "description": "Detailed explanation of this option"
                            },
                            "preview": {
                                "type": "string",
                                "description": "Optional preview content (markdown) rendered when focused"
                            }
                        },
                        "required": ["label", "description"]
                    }
                },
                "multiSelect": {
                    "type": "boolean",
                    "description": "Whether multiple answers can be selected (default: false)",
                    "default": false
                }
            },
            "required": ["question", "options"]
        })
    }

    /// The interactive execution is handled directly in the REPL layer.
    /// This stub returns an error so the REPL knows to intercept it.
    async fn execute(&self, _input: serde_json::Value) -> Result<ToolOutput, ToolError> {
        Err(ToolError {
            message: "ask_user_question requires interactive execution in the REPL".to_string(),
            code: Some("interactive_required".to_string()),
        })
    }
}
