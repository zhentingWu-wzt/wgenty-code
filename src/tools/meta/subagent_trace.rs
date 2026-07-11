//! Subagent Trace Tool — visualize subagent execution traces.
//!
//! Queries the SubagentTranscriptStore to render:
//!   - ASCII call tree with waterfall timing bars
//!   - Error timeline with failure mode breakdown
//!   - Chrome Trace Event Format JSON for external tools (Perfetto, DevTools)
//!
//! The tool is read-only and requires a session_id. When no session_id is
//! provided, it returns a usage hint.

use crate::agent::{AgentCoordinator, ToolContext};
use crate::teams::subagent_trace::SubagentTraceReporter;
use crate::tools::{Tool, ToolError, ToolOutput};
use async_trait::async_trait;
use std::sync::Arc;

pub struct SubagentTraceTool {
    store: Option<Arc<crate::transcript::SubagentTranscriptStore>>,
    coordinator: Arc<AgentCoordinator>,
}

impl SubagentTraceTool {
    pub fn new(
        store: Option<Arc<crate::transcript::SubagentTranscriptStore>>,
        coordinator: Arc<AgentCoordinator>,
    ) -> Self {
        Self { store, coordinator }
    }
}

#[async_trait]
impl Tool for SubagentTraceTool {
    fn name(&self) -> &str {
        "subagent_trace"
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn description(&self) -> &str {
        "Render subagent execution traces. Supports 'call_tree' (ASCII waterfall), \
         'error_timeline' (failure mode breakdown), and 'chrome_trace' (Perfetto-compatible JSON). \
         Requires a session_id. Use to diagnose subagent performance and failures."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "session_id": {
                    "type": "string",
                    "description": "The session ID to trace subagent activity for."
                },
                "format": {
                    "type": "string",
                    "enum": ["call_tree", "chrome_trace", "error_timeline", "html"],
                    "description": "Output format: 'call_tree' (ASCII tree+waterfall), 'chrome_trace' (JSON for Perfetto), 'error_timeline' (failure breakdown), or 'html' (self-contained HTML report). Default: 'call_tree'."
                }
            },
            "required": ["session_id"]
        })
    }

    async fn execute(&self, _input: serde_json::Value) -> Result<ToolOutput, ToolError> {
        Err(ToolError {
            message: "subagent_trace requires trusted agent context".to_string(),
            code: Some("missing_agent_context".to_string()),
        })
    }

    async fn execute_with_context(
        &self,
        context: &ToolContext<'_>,
        input: serde_json::Value,
    ) -> Result<ToolOutput, ToolError> {
        let store = self.store.as_ref().ok_or_else(|| ToolError {
            message: "Subagent transcript store is not available (persistence disabled)."
                .to_string(),
            code: Some("store_unavailable".to_string()),
        })?;

        let reporter = SubagentTraceReporter::new(store.clone());

        // Trusted session identity from the execution context. A raw
        // `session_id` in the input is ignored: it confers no authority. The
        // caller may only trace its own session.
        let session_id = context.agent.session_id.as_str();
        let _ = &self.coordinator; // authorization boundary lives in the coordinator

        let format = input["format"].as_str().unwrap_or("call_tree");

        match format {
            "chrome_trace" => {
                let json = reporter
                    .export_chrome_trace(session_id)
                    .map_err(|e| ToolError {
                        message: e,
                        code: Some("trace_export_failed".to_string()),
                    })?;
                Ok(ToolOutput {
                    output_type: "json".to_string(),
                    content: serde_json::to_string_pretty(&json).unwrap_or_default(),
                    metadata: std::collections::HashMap::from([(
                        "format".to_string(),
                        serde_json::Value::String("chrome_trace".to_string()),
                    )]),
                })
            }
            "error_timeline" => {
                let output = reporter
                    .render_error_timeline(
                        Some(session_id),
                        crate::teams::subagent_health::HealthPeriod::Last24h,
                    )
                    .map_err(|e| ToolError {
                        message: e,
                        code: Some("trace_error_timeline_failed".to_string()),
                    })?;
                Ok(ToolOutput {
                    output_type: "text".to_string(),
                    content: output,
                    metadata: std::collections::HashMap::from([(
                        "format".to_string(),
                        serde_json::Value::String("error_timeline".to_string()),
                    )]),
                })
            }
            "html" => {
                let output = reporter
                    .render_html_report(session_id)
                    .map_err(|e| ToolError {
                        message: e,
                        code: Some("trace_html_report_failed".to_string()),
                    })?;
                Ok(ToolOutput {
                    output_type: "html".to_string(),
                    content: output,
                    metadata: std::collections::HashMap::from([(
                        "format".to_string(),
                        serde_json::Value::String("html".to_string()),
                    )]),
                })
            }
            _ => {
                // Default: call_tree
                let output = reporter
                    .render_call_tree(session_id)
                    .map_err(|e| ToolError {
                        message: e,
                        code: Some("trace_render_failed".to_string()),
                    })?;
                Ok(ToolOutput {
                    output_type: "text".to_string(),
                    content: output,
                    metadata: std::collections::HashMap::from([(
                        "format".to_string(),
                        serde_json::Value::String("call_tree".to_string()),
                    )]),
                })
            }
        }
    }
}
