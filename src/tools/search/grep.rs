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
                    "description": "Glob patterns of files to include"
                },
                "exclude": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Glob patterns of files to exclude"
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
}

fn parse_patterns(value: &serde_json::Value) -> Vec<glob::Pattern> {
    value
        .as_array()
        .map(|values| {
            values
                .iter()
                .filter_map(|value| value.as_str())
                .filter_map(|pattern| glob::Pattern::new(pattern).ok())
                .collect()
        })
        .unwrap_or_default()
}

fn matches_patterns(path: &Path, include: &[glob::Pattern], exclude: &[glob::Pattern]) -> bool {
    let display = path.to_string_lossy();

    if !include.is_empty() && !include.iter().any(|pattern| pattern.matches(&display)) {
        return false;
    }

    if exclude.iter().any(|pattern| pattern.matches(&display)) {
        return false;
    }

    true
}

impl Default for GrepTool {
    fn default() -> Self {
        Self::new()
    }
}
