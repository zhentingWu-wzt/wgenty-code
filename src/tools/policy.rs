use super::{Tool, ToolError};
use crate::config::Settings;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub struct ToolPermissionPolicy {
    workspace_root: PathBuf,
}

pub enum PolicyDecision {
    Allow,
    Ask(PermissionRequest),
}

#[derive(Clone, Debug)]
pub struct PermissionRequest {
    pub tool_name: String,
    pub reason: String,
    pub session_rule: String,
}

impl ToolPermissionPolicy {
    pub fn from_settings(settings: &Settings) -> Self {
        let workspace_root = canonical_or_original(&settings.working_dir);
        Self { workspace_root }
    }

    pub fn validate_tool_call(
        &self,
        tool: &dyn Tool,
        tool_name: &str,
        args: &serde_json::Value,
        session_rules: &HashSet<String>,
    ) -> Result<PolicyDecision, ToolError> {
        if tool.is_read_only() {
            return Ok(PolicyDecision::Allow);
        }

        match tool_name {
            "file_write" | "file_edit" | "apply_patch" => {
                self.validate_write_paths(tool_name, args, session_rules)
            }
            "execute_command" | "exec_command" => {
                self.validate_command(tool_name, args, session_rules)
            }
            _ => Ok(PolicyDecision::Allow),
        }
    }

    fn validate_write_paths(
        &self,
        tool_name: &str,
        args: &serde_json::Value,
        session_rules: &HashSet<String>,
    ) -> Result<PolicyDecision, ToolError> {
        match tool_name {
            "file_write" | "file_edit" => {
                if let Some(path) = args["path"].as_str() {
                    if let Some(rule_key) = self.path_rule_key(path)? {
                        if session_rules.contains(&rule_key) {
                            return Ok(PolicyDecision::Allow);
                        }
                        return Ok(PolicyDecision::Ask(PermissionRequest {
                            tool_name: tool_name.to_string(),
                            reason: format!("write path is outside the workspace: {}", path),
                            session_rule: rule_key,
                        }));
                    }
                }
            }
            "apply_patch" => {
                let workdir = args["workdir"]
                    .as_str()
                    .map(PathBuf::from)
                    .unwrap_or_else(|| self.workspace_root.clone());
                let resolved = canonical_or_original(&workdir);
                if !resolved.starts_with(&self.workspace_root) {
                    let rule_key = format!("workdir:{}", resolved.display());
                    if session_rules.contains(&rule_key) {
                        return Ok(PolicyDecision::Allow);
                    }
                    return Ok(PolicyDecision::Ask(PermissionRequest {
                        tool_name: tool_name.to_string(),
                        reason: format!(
                            "apply_patch workdir is outside the workspace: {}",
                            resolved.display()
                        ),
                        session_rule: rule_key,
                    }));
                }
            }
            _ => {}
        }
        Ok(PolicyDecision::Allow)
    }

    fn validate_command(
        &self,
        tool_name: &str,
        args: &serde_json::Value,
        session_rules: &HashSet<String>,
    ) -> Result<PolicyDecision, ToolError> {
        let command = args["command"].as_str().ok_or_else(|| ToolError {
            message: format!("command is required for {}", tool_name),
            code: Some("missing_parameter".to_string()),
        })?;

        let denied_fragments = ["rm -rf", "mkfs", "shutdown", "reboot", ":(){:|:&};:"];
        if denied_fragments.iter().any(|fragment| command.contains(fragment)) {
            let rule_key = format!("command:{}", command_prefix(command));
            if session_rules.contains(&rule_key) {
                return Ok(PolicyDecision::Allow);
            }
            return Ok(PolicyDecision::Ask(PermissionRequest {
                tool_name: tool_name.to_string(),
                reason: format!("dangerous shell command requires approval: {}", command),
                session_rule: rule_key,
            }));
        }

        if let Some(workdir) = args["workdir"].as_str() {
            if let Some(rule_key) = self.path_rule_key(workdir)? {
                if session_rules.contains(&rule_key) {
                    return Ok(PolicyDecision::Allow);
                }
                return Ok(PolicyDecision::Ask(PermissionRequest {
                    tool_name: tool_name.to_string(),
                    reason: format!("workdir is outside the workspace: {}", workdir),
                    session_rule: rule_key,
                }));
            }
        }

        Ok(PolicyDecision::Allow)
    }

    fn path_rule_key(&self, raw_path: &str) -> Result<Option<String>, ToolError> {
        let path = PathBuf::from(raw_path);
        let resolved = if path.is_absolute() {
            canonical_or_original(&path)
        } else {
            canonical_or_original(&self.workspace_root.join(path))
        };

        if !resolved.starts_with(&self.workspace_root) {
            return Ok(Some(format!("path:{}", resolved.display())));
        }

        Ok(None)
    }
}

fn canonical_or_original(path: &Path) -> PathBuf {
    if let Ok(canon) = path.canonicalize() {
        return canon;
    }
    if let Some(parent) = path.parent() {
        if let Ok(canon_parent) = parent.canonicalize() {
            if let Some(filename) = path.file_name() {
                return canon_parent.join(filename);
            }
        }
    }
    path.to_path_buf()
}

fn command_prefix(command: &str) -> String {
    command
        .split_whitespace()
        .take(2)
        .collect::<Vec<_>>()
        .join(" ")
}
