//! File Edit Tool

use crate::agent::ToolContext;
use crate::tools::{Tool, ToolError, ToolOutput};
use async_trait::async_trait;
use serde_json;
use std::path::Path;

pub struct FileEditTool;

impl Default for FileEditTool {
    fn default() -> Self {
        Self::new()
    }
}

impl FileEditTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for FileEditTool {
    fn name(&self) -> &str {
        "file_edit"
    }

    fn description(&self) -> &str {
        "Edit a file by replacing specific content"
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to edit"
                },
                "old_content": {
                    "type": "string",
                    "description": "Content to find and replace"
                },
                "new_content": {
                    "type": "string",
                    "description": "New content to replace with"
                }
            },
            "required": ["path", "old_content", "new_content"]
        })
    }

    async fn execute_with_context(
        &self,
        context: &ToolContext<'_>,
        mut input: serde_json::Value,
    ) -> Result<ToolOutput, ToolError> {
        // s12: resolve relative paths against the per-agent workdir.
        if let Some(path) = input.get("path").and_then(|v| v.as_str()) {
            let resolved = crate::tools::resolve_path(path, context.workdir);
            input["path"] = serde_json::Value::String(resolved.to_string_lossy().into_owned());
        }
        self.execute(input).await
    }

    async fn execute(&self, input: serde_json::Value) -> Result<ToolOutput, ToolError> {
        let file_path = input["path"].as_str().ok_or_else(|| ToolError {
            message: "path is required".to_string(),
            code: Some("missing_parameter".to_string()),
        })?;

        let old_content = input["old_content"].as_str().ok_or_else(|| ToolError {
            message: "old_content is required".to_string(),
            code: Some("missing_parameter".to_string()),
        })?;

        let new_content = input["new_content"].as_str().ok_or_else(|| ToolError {
            message: "new_content is required".to_string(),
            code: Some("missing_parameter".to_string()),
        })?;

        let path = Path::new(file_path);

        if !path.exists() {
            return Err(ToolError {
                message: format!("File does not exist: {}", file_path),
                code: Some("file_not_found".to_string()),
            });
        }

        let old_file_content = std::fs::read_to_string(path).map_err(|e| ToolError {
            message: format!("Failed to read file: {}", e),
            code: Some("read_error".to_string()),
        })?;

        if !old_file_content.contains(old_content) {
            // 诊断日志: 记录匹配失败的关键上下文，便于定位 CRLF/缩进/末尾换行差异。
            // 运行时通过 RUST_LOG=wgenty_code::tools::filesystem::file_edit=debug 可见。
            tracing::debug!(
                target: "file_edit",
                path = %file_path,
                file_lines = old_file_content.lines().count(),
                file_has_crlf = old_file_content.contains('\r'),
                old_content_len = old_content.len(),
                old_content_has_crlf = old_content.contains('\r'),
                "old_content not found in file (exact substring match failed)"
            );
            // H1: 失败信息附带文件实际内容片段（带行号 + 行尾类型），
            // 让模型能定位差异、自我修正，而非转 python 全文替换。
            return Err(ToolError {
                message: format!(
                    "old_content not found in {} (file has {} lines, CRLF={}).\n\
                     File content (with line numbers):\n{}\n\
                     --- old_content first line: {:?} ---",
                    file_path,
                    old_file_content.lines().count(),
                    old_file_content.contains('\r'),
                    format_numbered_excerpt(&old_file_content),
                    old_content.lines().next().unwrap_or(""),
                ),
                code: Some("content_not_found".to_string()),
            });
        }

        // H2: 只替换第一处匹配，避免 String::replace 全量替换误伤重复片段。
        let new_file_content = old_file_content.replacen(old_content, new_content, 1);

        let write_result = std::fs::write(path, &new_file_content).map_err(|e| ToolError {
            message: format!("Failed to write file: {}", e),
            code: Some("write_error".to_string()),
        });

        let mut metadata = std::collections::HashMap::new();
        metadata.insert(
            "old_content".to_string(),
            serde_json::json!(old_file_content),
        );
        metadata.insert(
            "new_content".to_string(),
            serde_json::json!(new_file_content),
        );

        write_result?;

        Ok(ToolOutput {
            output_type: "text".to_string(),
            content: format!("Successfully edited {}", file_path),
            metadata,
        })
    }
}

/// 格式化文件内容片段（带行号），截断到合理长度以控制 token 开销。
/// 用于 file_edit 匹配失败时向模型提供可操作的文件上下文。
fn format_numbered_excerpt(content: &str) -> String {
    const MAX_LINES: usize = 30;
    const MAX_CHARS: usize = 2000;
    let total = content.lines().count();
    let mut out = String::new();
    let mut chars_used = 0usize;
    for (i, line) in content.lines().enumerate() {
        if i >= MAX_LINES {
            out.push_str(&format!(
                "  ... ({} more lines truncated)\n",
                total.saturating_sub(MAX_LINES)
            ));
            break;
        }
        let entry = format!("  {}: {}\n", i + 1, line);
        chars_used += entry.len();
        if chars_used > MAX_CHARS {
            out.push_str("  ... (truncated for length)\n");
            break;
        }
        out.push_str(&entry);
    }
    out.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 基线: 正常 LF 文件单处替换应工作（确认测试基础设施正确）。
    #[tokio::test]
    async fn edit_lf_baseline_replaces_single_occurrence() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("base.txt");
        std::fs::write(&path, "hello\nworld\n").unwrap();
        let tool = FileEditTool::new();
        tool.execute(serde_json::json!({
            "path": path.to_string_lossy(),
            "old_content": "world",
            "new_content": "rust",
        }))
        .await
        .unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello\nrust\n");
    }

    /// H1 修复: old_content 不匹配时，错误信息附带文件实际内容（带行号 + CRLF 状态），
    /// 让模型能定位差异、自我修正，而非转 python 全文替换。
    #[tokio::test]
    async fn h1_edit_error_includes_file_content_context() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("h1.txt");
        std::fs::write(&path, "line one\nline two\nline three\n").unwrap();
        let tool = FileEditTool::new();
        let err = tool
            .execute(serde_json::json!({
                "path": path.to_string_lossy(),
                "old_content": "line four",
                "new_content": "x",
            }))
            .await
            .unwrap_err();
        assert_eq!(err.code.as_deref(), Some("content_not_found"));
        assert!(err.message.contains("line one"), "got: {}", err.message);
        assert!(err.message.contains("line two"), "got: {}", err.message);
        assert!(err.message.contains("CRLF=false"), "got: {}", err.message);
    }

    /// H2 修复: 只替换第一处匹配，重复片段不被误改。
    #[tokio::test]
    async fn h2_edit_replaces_only_first_occurrence() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("h2.txt");
        std::fs::write(&path, "foo\nbar\nfoo\nbaz\n").unwrap();
        let tool = FileEditTool::new();
        tool.execute(serde_json::json!({
            "path": path.to_string_lossy(),
            "old_content": "foo",
            "new_content": "qux",
        }))
        .await
        .unwrap();
        let after = std::fs::read_to_string(&path).unwrap();
        // 只有第一处 foo 被替换。
        assert_eq!(after, "qux\nbar\nfoo\nbaz\n");
    }
}
