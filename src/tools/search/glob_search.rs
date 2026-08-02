use crate::tools::{Tool, ToolError, ToolOutput};
use async_trait::async_trait;
use std::collections::HashMap;
use std::path::Path;

pub struct GlobTool;

impl GlobTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for GlobTool {
    fn name(&self) -> &str {
        "glob"
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn description(&self) -> &str {
        "Find files matching glob patterns"
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Base path to search from"
                },
                "pattern": {
                    "type": "string",
                    "description": "Glob pattern to match"
                },
                "max_results": {
                    "type": "integer",
                    "description": "Maximum number of matches to return"
                }
            },
            "required": ["path", "pattern"]
        })
    }

    async fn execute(&self, input: serde_json::Value) -> Result<ToolOutput, ToolError> {
        let path = input["path"].as_str().ok_or_else(|| ToolError {
            message: "path is required".to_string(),
            code: Some("missing_parameter".to_string()),
        })?;
        let pattern = input["pattern"].as_str().ok_or_else(|| ToolError {
            message: "pattern is required".to_string(),
            code: Some("missing_parameter".to_string()),
        })?;
        let max_results = input["max_results"]
            .as_u64()
            .unwrap_or(200)
            .try_into()
            .unwrap_or(usize::MAX);

        let base = Path::new(path);
        if !base.exists() {
            return Err(ToolError {
                message: format!("Path does not exist: {}", path),
                code: Some("path_not_found".to_string()),
            });
        }

        let glob_pattern = glob::Pattern::new(pattern).map_err(|e| ToolError {
            message: format!("Invalid glob pattern: {}", e),
            code: Some("invalid_pattern".to_string()),
        })?;

        let mut results = Vec::new();
        let mut truncated = false;

        for entry in walkdir::WalkDir::new(base)
            .into_iter()
            .filter_map(|entry| entry.ok())
        {
            let entry_path = entry.path();
            let display = entry_path.to_string_lossy();
            if glob_pattern.matches(&display) {
                results.push(display.to_string());
                if results.len() >= max_results {
                    truncated = true;
                    break;
                }
            }
        }

        let mut metadata = HashMap::new();
        metadata.insert("result_count".to_string(), serde_json::json!(results.len()));
        metadata.insert("truncated".to_string(), serde_json::json!(truncated));

        Ok(ToolOutput {
            output_type: "text".to_string(),
            content: results.join("\n"),
            metadata,
        })
    }

    async fn execute_with_context(
        &self,
        context: &crate::agent::ToolContext<'_>,
        mut input: serde_json::Value,
    ) -> Result<ToolOutput, ToolError> {
        // s12: resolve the relative search root against the session's bound
        // worktree (same adapter as list_files).
        if let Some(path) = input.get("path").and_then(|v| v.as_str()) {
            let resolved = crate::tools::resolve_path(path, context.workdir);
            input["path"] = serde_json::Value::String(resolved.to_string_lossy().into_owned());
        }
        self.execute(input).await
    }
}

impl Default for GlobTool {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{AgentExecutionContext, SessionId, ToolContext, ToolInvocationId};
    use crate::tools::Tool;

    fn make_ctx<'a>(
        root: &'a AgentExecutionContext,
        wd: Option<&'a std::path::Path>,
    ) -> ToolContext<'a> {
        ToolContext {
            agent: root,
            invocation_id: ToolInvocationId::new("inv"),
            origin_turn_id: None,
            workdir: wd,
            effective_mode: crate::sandbox::EffectiveMode::default(),
            checkpoint: None,
        }
    }

    #[tokio::test]
    async fn glob_resolves_relative_path_against_workdir() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("hit.txt"), "x").unwrap();
        let root = AgentExecutionContext::root(SessionId::new("s"));
        let wd = dir.path().to_path_buf();
        let ctx = make_ctx(&root, Some(&wd));

        let out = GlobTool::new()
            .execute_with_context(&ctx, serde_json::json!({"pattern": "*.txt", "path": "."}))
            .await
            .unwrap();
        assert!(out.content.contains("hit.txt"), "{}", out.content);
    }
}
