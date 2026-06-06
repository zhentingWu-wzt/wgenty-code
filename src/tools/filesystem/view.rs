//! View Tool — Recursive directory listing with depth control

use crate::tools::{Tool, ToolError, ToolOutput};
use async_trait::async_trait;
use std::collections::HashMap;
use std::path::PathBuf;

/// Context bundled to keep [`walk_tree`] under 8-argument clippy limit.
struct WalkCtx<'a> {
    depth: usize,
    prefix: String,
    lines: &'a mut Vec<String>,
    count: &'a mut usize,
    skipped: &'a mut usize,
    offset: usize,
    limit: usize,
}

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
        let path = input["path"].as_str().unwrap_or(".");
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

        walk_tree(
            &base,
            WalkCtx {
                depth,
                prefix: String::new(),
                lines: &mut lines,
                count: &mut count,
                skipped: &mut skipped,
                offset,
                limit,
            },
        );

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

fn walk_tree(dir: &std::path::Path, ctx: WalkCtx) {
    if ctx.depth == 0 {
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
        b_is_dir
            .cmp(&a_is_dir)
            .then_with(|| a.file_name().cmp(&b.file_name()))
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
        if *ctx.count < ctx.offset {
            *ctx.skipped += 1;
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                walk_tree(
                    &entry.path(),
                    WalkCtx {
                        depth: ctx.depth - 1,
                        prefix: format!("{}{}", ctx.prefix, child_prefix),
                        lines: ctx.lines,
                        count: ctx.count,
                        skipped: ctx.skipped,
                        offset: ctx.offset,
                        limit: ctx.limit,
                    },
                );
            }
            continue;
        }
        if *ctx.count >= ctx.offset + ctx.limit {
            return;
        }

        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            ctx.lines.push(format!("{}{}{}/", ctx.prefix, connector, name));
            *ctx.count += 1;
            walk_tree(
                &entry.path(),
                WalkCtx {
                    depth: ctx.depth - 1,
                    prefix: format!("{}{}", ctx.prefix, if is_last { "    " } else { "│   " }),
                    lines: ctx.lines,
                    count: ctx.count,
                    skipped: ctx.skipped,
                    offset: ctx.offset,
                    limit: ctx.limit,
                },
            );
        } else {
            ctx.lines.push(format!("{}{}{}", ctx.prefix, connector, name));
            *ctx.count += 1;
        }
    }
}
