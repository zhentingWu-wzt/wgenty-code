use crate::tools::{Tool, ToolError, ToolOutput};
use async_trait::async_trait;
use regex::Regex;
use std::collections::HashMap;
use std::path::Path;

pub struct GrepTool;

impl GrepTool {
    pub fn new() -> Self {
        Self
    }

    pub fn run_search(input: &serde_json::Value) -> Result<ToolOutput, ToolError> {
        let path = input["path"].as_str().ok_or_else(|| ToolError {
            message: "path is required".to_string(),
            code: Some("missing_parameter".to_string()),
        })?;
        let pattern = input["pattern"].as_str().ok_or_else(|| ToolError {
            message: "pattern is required".to_string(),
            code: Some("missing_parameter".to_string()),
        })?;

        let include = parse_patterns(&input["include"]);
        let exclude = parse_patterns(&input["exclude"]);
        let max_results = input["max_results"]
            .as_u64()
            .unwrap_or(200)
            .try_into()
            .unwrap_or(usize::MAX);
        let files_with_matches = input["files_with_matches"].as_bool().unwrap_or(false);

        let base = Path::new(path);
        if !base.exists() {
            return Err(ToolError {
                message: format!("Path does not exist: {}", path),
                code: Some("path_not_found".to_string()),
            });
        }

        let regex = Regex::new(pattern).map_err(|e| ToolError {
            message: format!("Invalid regex pattern: {}", e),
            code: Some("invalid_pattern".to_string()),
        })?;

        let mut matches: Vec<String> = Vec::new();
        let mut truncated = false;

        for entry in walkdir::WalkDir::new(base)
            .into_iter()
            .filter_map(|entry| entry.ok())
        {
            let entry_path = entry.path();
            if !entry_path.is_file() {
                continue;
            }

            if !matches_patterns(entry_path, &include, &exclude) {
                continue;
            }

            let Ok(content) = std::fs::read_to_string(entry_path) else {
                continue;
            };

            if files_with_matches {
                // --files-with-matches mode: only file paths with match counts
                let count = content.lines().filter(|l| regex.is_match(l)).count();
                if count > 0 {
                    matches.push(format!("{} ({} matches)", entry_path.display(), count));
                    if matches.len() >= max_results {
                        truncated = true;
                        break;
                    }
                }
            } else {
                for (line_num, line) in content.lines().enumerate() {
                    if regex.is_match(line) {
                        // Truncate long lines to keep output compact
                        let display_line = if line.chars().count() > 200 {
                            format!("{}…[truncated]", line.chars().take(200).collect::<String>())
                        } else {
                            line.to_string()
                        };
                        matches.push(format!(
                            "{}:{}: {}",
                            entry_path.display(),
                            line_num + 1,
                            display_line
                        ));
                        if matches.len() >= max_results {
                            truncated = true;
                            break;
                        }
                    }
                }
                if truncated {
                    break;
                }
            }
        }

        let mut metadata = HashMap::new();
        metadata.insert("result_count".to_string(), serde_json::json!(matches.len()));
        metadata.insert("truncated".to_string(), serde_json::json!(truncated));

        Ok(ToolOutput {
            output_type: "text".to_string(),
            content: matches.join("\n"),
            metadata,
        })
    }
}

#[async_trait]
impl Tool for GrepTool {
    fn name(&self) -> &str {
        "grep"
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn description(&self) -> &str {
        "Search file contents using regex with include/exclude filters"
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Base path to search"
                },
                "pattern": {
                    "type": "string",
                    "description": "Regex pattern to match"
                },
                "include": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Glob patterns of files to include (supports {a,b} alternation)"
                },
                "exclude": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Glob patterns of files to exclude (supports {a,b} alternation)"
                },
                "max_results": {
                    "type": "integer",
                    "description": "Maximum number of matching lines to return"
                },
                "files_with_matches": {
                    "type": "boolean",
                    "description": "Only show file paths with match counts, not individual lines. Useful for scoping searches before diving into details."
                }
            },
            "required": ["path", "pattern"]
        })
    }

    async fn execute(&self, input: serde_json::Value) -> Result<ToolOutput, ToolError> {
        Self::run_search(&input)
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

fn parse_patterns(value: &serde_json::Value) -> Vec<globset::GlobMatcher> {
    value
        .as_array()
        .map(|values| {
            values
                .iter()
                .filter_map(|value| value.as_str())
                .filter_map(|pattern| {
                    globset::Glob::new(pattern)
                        .ok()
                        .map(|glob| glob.compile_matcher())
                })
                .collect()
        })
        .unwrap_or_default()
}

fn matches_patterns(
    path: &Path,
    include: &[globset::GlobMatcher],
    exclude: &[globset::GlobMatcher],
) -> bool {
    let display = path.to_string_lossy();
    let file_name = path
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_default();
    // Match against both the full path and the basename so that include
    // patterns like "policy.rs" match "src/permissions/policy.rs".
    let matches_any = |matcher: &globset::GlobMatcher| {
        matcher.is_match(display.as_ref()) || matcher.is_match(file_name.as_str())
    };

    if !include.is_empty() && !include.iter().any(matches_any) {
        return false;
    }

    if exclude.iter().any(matches_any) {
        return false;
    }

    true
}

impl Default for GrepTool {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{AgentExecutionContext, SessionId, ToolContext, ToolInvocationId};

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
    async fn grep_resolves_relative_path_against_workdir() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("hit.txt"), "hello needle\n").unwrap();
        let root = AgentExecutionContext::root(SessionId::new("s"));
        let wd = dir.path().to_path_buf();
        let ctx = make_ctx(&root, Some(&wd));

        let out = GrepTool::new()
            .execute_with_context(&ctx, serde_json::json!({"pattern": "needle", "path": "."}))
            .await
            .unwrap();
        assert!(out.content.contains("hit.txt"), "{}", out.content);
    }

    #[tokio::test]
    async fn grep_include_exclude_support_brace_alternation() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "needle\n").unwrap();
        std::fs::write(dir.path().join("b.md"), "needle\n").unwrap();
        std::fs::write(dir.path().join("c.rs"), "needle\n").unwrap();
        std::fs::write(dir.path().join("skip.txt"), "needle\n").unwrap();
        let root = AgentExecutionContext::root(SessionId::new("s"));
        let wd = dir.path().to_path_buf();
        let ctx = make_ctx(&root, Some(&wd));

        let out = GrepTool::new()
            .execute_with_context(
                &ctx,
                serde_json::json!({
                    "pattern": "needle",
                    "path": ".",
                    "include": ["*.{txt,md}"],
                    "exclude": ["skip.{txt,rs}"]
                }),
            )
            .await
            .unwrap();
        assert!(out.content.contains("a.txt"), "{}", out.content);
        assert!(out.content.contains("b.md"), "{}", out.content);
        assert!(!out.content.contains("c.rs"), "{}", out.content);
        assert!(!out.content.contains("skip.txt"), "{}", out.content);
    }
}
