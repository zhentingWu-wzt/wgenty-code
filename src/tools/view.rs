//! View Tool — Recursive directory listing with depth control

use super::{Tool, ToolError, ToolOutput};
use async_trait::async_trait;
use std::collections::HashMap;
use std::path::PathBuf;

pub struct ViewTool;

impl ViewTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for ViewTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for ViewTool {
    fn name(&self) -> &str {
        "view"
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn description(&self) -> &str {
        "View a directory as a tree. Use this to understand project structure quickly."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Directory path to view. Defaults to current directory."
                },
                "depth": {
                    "type": "integer",
                    "description": "Maximum recursion depth (default: 3, max: 10)",
                    "default": 3,
                    "minimum": 1,
                    "maximum": 10
                },
                "offset": {
                    "type": "integer",
                    "description": "Skip this many entries (for pagination)",
                    "default": 0
                },
                "limit": {
                    "type": "integer",
                    "description": "Max entries to return (default: 200)",
                    "default": 200
                }
            },
            "required": []
        })
    }

    async fn execute(&self, input: serde_json::Value) -> Result<ToolOutput, ToolError> {
        let path = input["path"]
            .as_str()
            .unwrap_or(".");
        let depth = input["depth"].as_u64().unwrap_or(3).min(10) as usize;
        let offset = input["offset"].as_u64().unwrap_or(0) as usize;
        let limit = input["limit"].as_u64().unwrap_or(200) as usize;

        let base = PathBuf::from(path);
        if !base.exists() {
            return Err(ToolError {
                message: format!("Path does not exist: {}", path),
                code: Some("path_not_found".to_string()),
            });
        }

        let mut lines: Vec<String> = Vec::new();
        let mut count: usize = 0;
        let mut skipped: usize = 0;

        lines.push(format!("{}", base.display()));

        walk_tree(&base, depth, "", &mut lines, &mut count, &mut skipped, offset, limit);

        if skipped > 0 {
            lines.push(format!("...[{} entries skipped]...", skipped));
        }
        if count > offset + limit {
            lines.push("...[truncated]".to_string());
        }

        Ok(ToolOutput {
            output_type: "text".to_string(),
            content: lines.join("\n"),
            metadata: HashMap::new(),
        })
    }
}

fn walk_tree(
    dir: &std::path::Path,
    depth: usize,
    prefix: &str,
    lines: &mut Vec<String>,
    count: &mut usize,
    skipped: &mut usize,
    offset: usize,
    limit: usize,
) {
    if depth == 0 {
        return;
    }

    let entries = match std::fs::read_dir(dir) {
        Ok(it) => it.filter_map(|e| e.ok()).collect::<Vec<_>>(),
        Err(_) => return,
    };

    let mut sorted = entries;
    sorted.sort_by(|a, b| {
        let a_is_dir = a.file_type().map(|t| t.is_dir()).unwrap_or(false);
        let b_is_dir = b.file_type().map(|t| t.is_dir()).unwrap_or(false);
        b_is_dir.cmp(&a_is_dir).then_with(|| {
            a.file_name().cmp(&b.file_name())
        })
    });

    // Skip hidden files/dirs (starting with .)
    let visible: Vec<_> = sorted
        .into_iter()
        .filter(|e| {
            e.file_name()
                .to_str()
                .map(|n| !n.starts_with('.'))
                .unwrap_or(false)
        })
        .collect();

    for (i, entry) in visible.iter().enumerate() {
        let is_last = i == visible.len() - 1;
        let connector = if is_last { "└── " } else { "├── " };
        let child_prefix = if is_last { "    " } else { "│   " };

        let name = entry.file_name().to_string_lossy().to_string();

        // Respect offset/limit
        if *count < offset {
            *skipped += 1;
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                walk_tree(&entry.path(), depth - 1, &format!("{}{}", prefix, child_prefix), lines, count, skipped, offset, limit);
            }
            continue;
        }
        if *count >= offset + limit {
            return;
        }

        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            lines.push(format!("{}{}{}/", prefix, connector, name));
            *count += 1;
            walk_tree(
                &entry.path(),
                depth - 1,
                &format!("{}{}", prefix, if is_last { "    " } else { "│   " }),
                lines,
                count,
                skipped,
                offset,
                limit,
            );
        } else {
            lines.push(format!("{}{}{}", prefix, connector, name));
            *count += 1;
        }
    }
}
