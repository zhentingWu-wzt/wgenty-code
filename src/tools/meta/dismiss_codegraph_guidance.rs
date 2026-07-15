//! Dismiss CodeGraph install/init guidance for the current project.
//!
//! Persists the current working dir into
//! `settings.integrations.codegraph.dismissed_paths` (canonicalized, deduped)
//! so the CLI startup notice and the agent's on-demand ask go silent for this
//! project. Invoked by the agent when the user picks "don't remind again" in
//! the codegraph guidance `ask_user_question` flow.

use crate::tools::{Tool, ToolError, ToolOutput};
use async_trait::async_trait;
use std::path::{Path, PathBuf};

pub struct DismissCodegraphGuidanceTool;

impl Default for DismissCodegraphGuidanceTool {
    fn default() -> Self {
        Self::new()
    }
}

impl DismissCodegraphGuidanceTool {
    pub fn new() -> Self {
        Self
    }
}

/// Canonicalize `working_dir` and push into `paths` if not already present.
/// Returns the canonical path that was ensured present. Dedupes by canonical
/// form so symlinks / `..` segments don't create duplicate entries.
pub fn add_dismissed_path(paths: &mut Vec<PathBuf>, working_dir: &Path) -> PathBuf {
    let canon = std::fs::canonicalize(working_dir).unwrap_or_else(|_| working_dir.to_path_buf());
    let already = paths
        .iter()
        .any(|p| std::fs::canonicalize(p).unwrap_or_else(|_| p.clone()) == canon);
    if !already {
        paths.push(canon.clone());
    }
    canon
}

#[async_trait]
impl Tool for DismissCodegraphGuidanceTool {
    fn name(&self) -> &str {
        "dismiss_codegraph_guidance"
    }

    fn description(&self) -> &str {
        "Silence CodeGraph install/initialization guidance for the current project. \
         Persists the working directory to settings so the startup notice and \
         on-demand prompts no longer appear. Use when the user chooses not to \
         install CodeGraph."
    }

    fn is_read_only(&self) -> bool {
        // Writes to ~/.wgenty-code/settings.json.
        false
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Optional working directory to dismiss (defaults to current working dir)."
                }
            },
            "required": []
        })
    }

    async fn execute(&self, input: serde_json::Value) -> Result<ToolOutput, ToolError> {
        let path = input["path"]
            .as_str()
            .map(PathBuf::from)
            .or_else(|| std::env::current_dir().ok())
            .ok_or_else(|| ToolError {
                message: "could not resolve working directory".to_string(),
                code: Some("no_cwd".to_string()),
            })?;

        let mut settings = crate::config::Settings::load().map_err(|e| ToolError {
            message: format!("failed to load settings: {e}"),
            code: Some("settings_load".to_string()),
        })?;
        let canon = add_dismissed_path(&mut settings.integrations.codegraph.dismissed_paths, &path);
        settings.save().map_err(|e| ToolError {
            message: format!("failed to save settings: {e}"),
            code: Some("settings_save".to_string()),
        })?;
        Ok(ToolOutput::text(format!(
            "CodeGraph guidance dismissed for {}",
            canon.display()
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn add_dismissed_path_dedups() {
        let tmp = tempfile::tempdir().unwrap();
        let mut paths: Vec<PathBuf> = Vec::new();
        add_dismissed_path(&mut paths, tmp.path());
        add_dismissed_path(&mut paths, tmp.path());
        assert_eq!(paths.len(), 1);
    }

    #[test]
    fn add_dismissed_path_adds_new() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let mut paths: Vec<PathBuf> = Vec::new();
        add_dismissed_path(&mut paths, a.path());
        add_dismissed_path(&mut paths, b.path());
        assert_eq!(paths.len(), 2);
    }
}
