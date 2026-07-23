//! Memory Add Tool
//!
//! Lets the agent proactively write a memory entry to persistent storage
//! via `MemoryManager::add_memory()`, without waiting for context compaction.
//! Supports scope selection (project/global), memory type, tags, and importance.

use crate::context::{MemoryEntry, MemoryManager, MemoryOrigin, MemoryType};
use crate::tools::{Tool, ToolError, ToolOutput};
use async_trait::async_trait;
use std::sync::Arc;

pub struct MemoryAddTool {
    memory: Arc<MemoryManager>,
}

impl MemoryAddTool {
    pub fn new(memory: Arc<MemoryManager>) -> Self {
        Self { memory }
    }

    fn parse_memory_type(s: &str) -> Result<MemoryType, ToolError> {
        match s {
            "Knowledge" => Ok(MemoryType::Knowledge),
            "Preference" => Ok(MemoryType::Preference),
            "Session" => Ok(MemoryType::Session),
            "Conversation" => Ok(MemoryType::Conversation),
            "Task" => Ok(MemoryType::Task),
            "Error" => Ok(MemoryType::Error),
            "Insight" => Ok(MemoryType::Insight),
            "Decision" => Ok(MemoryType::Decision),
            _ => Err(ToolError {
                message: format!(
                    "Invalid memory_type '{}'. Must be one of: Knowledge, Preference, Session, Conversation, Task, Error, Insight, Decision",
                    s
                ),
                code: Some("invalid_memory_type".to_string()),
            }),
        }
    }

    fn parse_scope(s: &str) -> Result<MemoryOrigin, ToolError> {
        match s {
            "project" => Ok(MemoryOrigin::Project),
            "global" => Ok(MemoryOrigin::Global),
            _ => Err(ToolError {
                message: format!("Invalid scope '{}'. Must be 'project' or 'global'", s),
                code: Some("invalid_scope".to_string()),
            }),
        }
    }
}

#[async_trait]
impl Tool for MemoryAddTool {
    fn name(&self) -> &str {
        "memory_add"
    }

    fn description(&self) -> &str {
        "Write a memory entry (lesson, decision, preference, fact) to persistent storage - but ONLY after it passes the veto gate: (1) Durable - useful in weeks/months, not this task; (2) Non-redundant - not already the source of truth in a skill/script/code/AGENTS.md/doc; (3) Retrieval-justified - next time you'll actually retrieve it and be glad, vs re-derive or load the authoritative source; (4) Insight not instance - a generalizable principle, not a single-event replay; (5) Dense - 1-3 sentences; if it needs a step-by-step recipe it belongs in a note/doc. Effort-to-acquire is not retention value. If borderline or unsure, ask the user before writing instead of defaulting to capture. Specify scope: 'project' for project-specific content (architecture, paths, conventions) or 'global' for cross-project insights (user preferences, workflow habits). Reuses the same dedup/merge logic as context compaction."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "content": {
                    "type": "string",
                    "description": "The memory content to persist"
                },
                "memory_type": {
                    "type": "string",
                    "description": "Memory type category",
                    "enum": ["Knowledge", "Preference", "Session", "Conversation", "Task", "Error", "Insight", "Decision"],
                    "default": "Knowledge"
                },
                "scope": {
                    "type": "string",
                    "description": "Storage scope: 'project' for project-specific, 'global' for cross-project",
                    "enum": ["project", "global"],
                    "default": "project"
                },
                "tags": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional tags for categorization"
                },
                "importance": {
                    "type": "number",
                    "description": "Importance score (0.0-1.0)",
                    "default": 0.5,
                    "minimum": 0.0,
                    "maximum": 1.0
                }
            },
            "required": ["content"]
        })
    }

    fn is_read_only(&self) -> bool {
        false
    }

    async fn execute(&self, input: serde_json::Value) -> Result<ToolOutput, ToolError> {
        let content = input["content"].as_str().ok_or_else(|| ToolError {
            message: "content is required".to_string(),
            code: Some("missing_content".to_string()),
        })?;

        let memory_type = match input["memory_type"].as_str() {
            Some(s) => Self::parse_memory_type(s)?,
            None => MemoryType::Knowledge,
        };

        let scope = match input["scope"].as_str() {
            Some(s) => Self::parse_scope(s)?,
            None => MemoryOrigin::Project,
        };

        let importance = input["importance"].as_f64().unwrap_or(0.5) as f32;

        let tags: Vec<String> = input["tags"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        let entry = MemoryEntry::new(memory_type, content)
            .with_importance(importance)
            .with_tags(tags);

        let result = self
            .memory
            .add_memory(entry, scope)
            .await
            .map_err(|e| ToolError {
                message: format!("Failed to add memory: {}", e),
                code: Some("memory_error".to_string()),
            })?;

        Ok(ToolOutput {
            output_type: "json".to_string(),
            content: serde_json::json!({
                "success": true,
                "memory_id": result.id,
                "merged": result.merged
            })
            .to_string(),
            metadata: std::collections::HashMap::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_mm(tmp: &tempfile::TempDir) -> Arc<MemoryManager> {
        Arc::new(MemoryManager::new(tmp.path().to_path_buf()))
    }

    #[tokio::test]
    async fn is_read_only_returns_false() {
        let tmp = tempfile::tempdir().unwrap();
        let tool = MemoryAddTool::new(make_mm(&tmp));
        assert!(!tool.is_read_only());
    }

    #[tokio::test]
    async fn creates_new_project_memory() {
        let tmp = tempfile::tempdir().unwrap();
        let tool = MemoryAddTool::new(make_mm(&tmp));

        let input = serde_json::json!({
            "content": "The config file is at ~/.wgenty-code/settings.json",
            "scope": "project"
        });

        let result = tool.execute(input).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(parsed["success"], true);
        assert_eq!(parsed["merged"], false);
        assert!(parsed["memory_id"].is_string());
    }

    #[tokio::test]
    async fn creates_new_global_memory() {
        // Isolate BOTH project and global storage so the test never writes to
        // the real `~/.wgenty-code/memory/` global pool (which previously
        // polluted the user's global memories on every `cargo test` run).
        let project_tmp = tempfile::tempdir().unwrap();
        let global_tmp = tempfile::tempdir().unwrap();
        let mm = Arc::new(MemoryManager::new_for_test(
            project_tmp.path().to_path_buf(),
            global_tmp.path().to_path_buf(),
        ));
        let tool = MemoryAddTool::new(mm);

        let input = serde_json::json!({
            "content": "test global scope parsing",
            "scope": "global"
        });

        let result = tool.execute(input).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(parsed["success"], true);

        // The global memory must land in the isolated dir, not the real one.
        let global_files: Vec<_> = std::fs::read_dir(global_tmp.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("json"))
            .collect();
        assert_eq!(
            global_files.len(),
            1,
            "global memory should be persisted to the isolated global dir"
        );
    }

    #[tokio::test]
    async fn similar_content_triggers_merge() {
        let tmp = tempfile::tempdir().unwrap();
        let mm = make_mm(&tmp);
        let tool = MemoryAddTool::new(mm.clone());

        // First: add a memory.
        let input1 = serde_json::json!({
            "content": "use JWT for authentication",
            "memory_type": "Decision"
        });
        tool.execute(input1).await.unwrap();

        // Second: similar content triggers dedup merge.
        let input2 = serde_json::json!({
            "content": "use JWT",
            "memory_type": "Knowledge"
        });
        let result = tool.execute(input2).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(parsed["success"], true);
        assert_eq!(parsed["merged"], true);
    }

    #[tokio::test]
    async fn missing_content_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        let tool = MemoryAddTool::new(make_mm(&tmp));

        let input = serde_json::json!({
            "memory_type": "Knowledge"
        });

        let err = tool.execute(input).await.unwrap_err();
        assert_eq!(err.code, Some("missing_content".to_string()));
    }

    #[tokio::test]
    async fn invalid_memory_type_returns_error() {
        let tmp = tempfile::tempdir().unwrap();
        let tool = MemoryAddTool::new(make_mm(&tmp));

        let input = serde_json::json!({
            "content": "test",
            "memory_type": "InvalidType"
        });

        let err = tool.execute(input).await.unwrap_err();
        assert_eq!(err.code, Some("invalid_memory_type".to_string()));
    }
}
