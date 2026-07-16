//! Git Operations Tool
//!
//! Provides Git version control operations including:
//! - git status: Show working tree status
//! - git add: Add files to staging area
//! - git commit: Create a new commit
//! - git push: Push to remote repository
//! - git pull: Pull from remote repository
//! - git log: Show commit history
//! - git diff: Show changes
//! - git branch: Manage branches
//! - git checkout: Switch branches or restore files

use crate::tools::{Tool, ToolError, ToolOutput};
use async_trait::async_trait;
use serde_json;
use std::path::Path;

pub struct GitOperationsTool;

impl Default for GitOperationsTool {
    fn default() -> Self {
        Self::new()
    }
}

impl GitOperationsTool {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Tool for GitOperationsTool {
    fn name(&self) -> &str {
        "git_operations"
    }

    fn description(&self) -> &str {
        "Execute Git version control operations"
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "description": "Git operation to perform: status, add, commit, push, pull, log, diff, branch, checkout, worktree_add, worktree_remove, worktree_list",
                    "enum": [
                        "status", "add", "commit", "push", "pull", "log", "diff",
                        "branch", "checkout", "worktree_add", "worktree_remove",
                        "worktree_list"
                    ]
                },
                "path": {
                    "type": "string",
                    "description": "Path to the git repository (optional, defaults to current directory). For worktree_add this is the worktree path."
                },
                "args": {
                    "type": "array",
                    "items": {
                        "type": "string"
                    },
                    "description": "Additional arguments for the git command"
                },
                "message": {
                    "type": "string",
                    "description": "Commit message (required for commit operation)"
                },
                "files": {
                    "type": "array",
                    "items": {
                        "type": "string"
                    },
                    "description": "Files to add (for add operation)"
                },
                "branch": {
                    "type": "string",
                    "description": "Branch name (for branch, checkout, push, pull, worktree_add operations)"
                },
                "base_ref": {
                    "type": "string",
                    "description": "Base reference to branch from (for worktree_add operation)"
                },
                "remote": {
                    "type": "string",
                    "description": "Remote name (defaults to 'origin')"
                },
                "force": {
                    "type": "boolean",
                    "description": "Force the operation (for worktree_remove)"
                }
            },
            "required": ["operation"]
        })
    }

    async fn execute(&self, input: serde_json::Value) -> Result<ToolOutput, ToolError> {
        let operation = input["operation"].as_str().ok_or_else(|| ToolError {
            message: "operation is required".to_string(),
            code: Some("missing_parameter".to_string()),
        })?;

        let path = input["path"].as_str().unwrap_or(".");
        let args: Vec<String> = input["args"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        // Change to the git repository directory
        let repo_path = Path::new(path);

        match operation {
            "status" => self.git_status(repo_path, &args).await,
            "add" => self.git_add(repo_path, &input, &args).await,
            "commit" => self.git_commit(repo_path, &input, &args).await,
            "push" => self.git_push(repo_path, &input, &args).await,
            "pull" => self.git_pull(repo_path, &input, &args).await,
            "log" => self.git_log(repo_path, &args).await,
            "diff" => self.git_diff(repo_path, &args).await,
            "branch" => self.git_branch(repo_path, &input, &args).await,
            "checkout" => self.git_checkout(repo_path, &input, &args).await,
            "worktree_add" => self.worktree_add(repo_path, &input).await,
            "worktree_remove" => self.worktree_remove(repo_path, &input).await,
            "worktree_list" => self.worktree_list(repo_path).await,
            _ => Err(ToolError {
                message: format!("Unknown git operation: {}", operation),
                code: Some("invalid_operation".to_string()),
            }),
        }
    }
}

impl GitOperationsTool {
    async fn execute_git_command(
        &self,
        repo_path: &Path,
        args: &[String],
    ) -> Result<ToolOutput, ToolError> {
        let output = tokio::process::Command::new("git")
            .current_dir(repo_path)
            .args(args)
            .output()
            .await
            .map_err(|e| ToolError {
                message: format!("Failed to execute git command: {}", e),
                code: Some("git_error".to_string()),
            })?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        if output.status.success() {
            // 成功: 优先 stdout; 某些 git 命令（如 clone）把进度信息写到 stderr。
            let content = if stdout.is_empty() && !stderr.is_empty() {
                stderr
            } else {
                stdout
            };
            Ok(ToolOutput {
                output_type: "text".to_string(),
                content,
                metadata: std::collections::HashMap::new(),
            })
        } else {
            // 非零退出码: 返回 Err，message 透传退出码 + stdout + stderr，
            // 让模型能看到 git 真实错误（冲突/rejected/参数错误等）并据此修正。
            Err(ToolError {
                message: format!(
                    "Git command failed with status {}\n--- stdout ---\n{}\n--- stderr ---\n{}",
                    output.status, stdout, stderr
                ),
                code: Some("git_command_failed".to_string()),
            })
        }
    }

    async fn git_status(&self, repo_path: &Path, args: &[String]) -> Result<ToolOutput, ToolError> {
        let mut cmd_args = vec!["status".to_string()];
        cmd_args.extend_from_slice(args);
        self.execute_git_command(repo_path, &cmd_args).await
    }

    async fn git_add(
        &self,
        repo_path: &Path,
        input: &serde_json::Value,
        args: &[String],
    ) -> Result<ToolOutput, ToolError> {
        let files: Vec<String> = input["files"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_else(|| vec![".".to_string()]); // Default to add all

        let mut cmd_args = vec!["add".to_string()];
        cmd_args.extend_from_slice(args);
        cmd_args.extend(files);

        self.execute_git_command(repo_path, &cmd_args).await
    }

    async fn git_commit(
        &self,
        repo_path: &Path,
        input: &serde_json::Value,
        args: &[String],
    ) -> Result<ToolOutput, ToolError> {
        let message = input["message"].as_str().ok_or_else(|| ToolError {
            message: "Commit message is required".to_string(),
            code: Some("missing_message".to_string()),
        })?;

        let mut cmd_args = vec!["commit".to_string(), "-m".to_string(), message.to_string()];
        cmd_args.extend_from_slice(args);

        self.execute_git_command(repo_path, &cmd_args).await
    }

    async fn git_push(
        &self,
        repo_path: &Path,
        input: &serde_json::Value,
        args: &[String],
    ) -> Result<ToolOutput, ToolError> {
        let remote = input["remote"].as_str().unwrap_or("origin");
        let branch = input["branch"].as_str().unwrap_or("");

        let mut cmd_args = vec!["push".to_string()];
        cmd_args.extend_from_slice(args);

        if !branch.is_empty() {
            cmd_args.push(format!("{}:{}", remote, branch));
        } else {
            cmd_args.push(remote.to_string());
        }

        self.execute_git_command(repo_path, &cmd_args).await
    }

    async fn git_pull(
        &self,
        repo_path: &Path,
        input: &serde_json::Value,
        args: &[String],
    ) -> Result<ToolOutput, ToolError> {
        let remote = input["remote"].as_str().unwrap_or("origin");
        let branch = input["branch"].as_str().unwrap_or("");

        let mut cmd_args = vec!["pull".to_string()];
        cmd_args.extend_from_slice(args);

        if !branch.is_empty() {
            cmd_args.push(remote.to_string());
            cmd_args.push(branch.to_string());
        }

        self.execute_git_command(repo_path, &cmd_args).await
    }

    async fn git_log(&self, repo_path: &Path, args: &[String]) -> Result<ToolOutput, ToolError> {
        let mut cmd_args = vec!["log".to_string()];
        cmd_args.extend_from_slice(args);

        // Add pretty format if no custom format specified
        if !args
            .iter()
            .any(|arg| arg.starts_with("--pretty=") || arg == "--oneline")
        {
            cmd_args.push("--oneline".to_string());
        }

        self.execute_git_command(repo_path, &cmd_args).await
    }

    async fn git_diff(&self, repo_path: &Path, args: &[String]) -> Result<ToolOutput, ToolError> {
        let mut cmd_args = vec!["diff".to_string()];
        cmd_args.extend_from_slice(args);
        self.execute_git_command(repo_path, &cmd_args).await
    }

    async fn git_branch(
        &self,
        repo_path: &Path,
        input: &serde_json::Value,
        args: &[String],
    ) -> Result<ToolOutput, ToolError> {
        let branch = input["branch"].as_str();

        let mut cmd_args = vec!["branch".to_string()];
        cmd_args.extend_from_slice(args);

        if let Some(branch_name) = branch {
            cmd_args.push(branch_name.to_string());
        }

        self.execute_git_command(repo_path, &cmd_args).await
    }

    async fn git_checkout(
        &self,
        repo_path: &Path,
        input: &serde_json::Value,
        args: &[String],
    ) -> Result<ToolOutput, ToolError> {
        let branch = input["branch"].as_str().ok_or_else(|| ToolError {
            message: "Branch name is required for checkout".to_string(),
            code: Some("missing_branch".to_string()),
        })?;

        let mut cmd_args = vec!["checkout".to_string(), branch.to_string()];
        cmd_args.extend_from_slice(args);

        self.execute_git_command(repo_path, &cmd_args).await
    }

    /// Add a new git worktree.
    ///
    /// Creates a worktree at the given `path` on the specified `branch`,
    /// optionally branching from `base_ref`.
    async fn worktree_add(
        &self,
        repo_path: &Path,
        input: &serde_json::Value,
    ) -> Result<ToolOutput, ToolError> {
        let path = input["path"].as_str().ok_or_else(|| ToolError {
            message: "path is required for worktree_add".to_string(),
            code: Some("missing_parameter".to_string()),
        })?;
        let branch = input["branch"].as_str().ok_or_else(|| ToolError {
            message: "branch is required for worktree_add".to_string(),
            code: Some("missing_parameter".to_string()),
        })?;

        let mut cmd_args = vec!["worktree".to_string(), "add".to_string()];

        // If base_ref is provided, add it after -b <branch>
        if let Some(base_ref) = input["base_ref"].as_str().filter(|s| !s.is_empty()) {
            cmd_args.push("-b".to_string());
            cmd_args.push(branch.to_string());
            cmd_args.push(path.to_string());
            cmd_args.push(base_ref.to_string());
        } else {
            cmd_args.push("-b".to_string());
            cmd_args.push(branch.to_string());
            cmd_args.push(path.to_string());
        }

        self.execute_git_command(repo_path, &cmd_args).await
    }

    /// Remove a git worktree.
    ///
    /// Removes the worktree at the given `path`. If `force` is true, uses
    /// `git worktree remove --force`.
    async fn worktree_remove(
        &self,
        repo_path: &Path,
        input: &serde_json::Value,
    ) -> Result<ToolOutput, ToolError> {
        let path = input["path"].as_str().ok_or_else(|| ToolError {
            message: "path is required for worktree_remove".to_string(),
            code: Some("missing_parameter".to_string()),
        })?;

        let mut cmd_args = vec!["worktree".to_string(), "remove".to_string()];

        let force = input["force"].as_bool().unwrap_or(false);
        if force {
            cmd_args.push("--force".to_string());
        }

        cmd_args.push(path.to_string());

        self.execute_git_command(repo_path, &cmd_args).await
    }

    /// List all git worktrees.
    ///
    /// Runs `git worktree list` in the repository root.
    async fn worktree_list(&self, repo_path: &Path) -> Result<ToolOutput, ToolError> {
        let cmd_args = vec!["worktree".to_string(), "list".to_string()];
        self.execute_git_command(repo_path, &cmd_args).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_schema_contains_worktree_operations() {
        let tool = GitOperationsTool::new();
        let schema = tool.input_schema();

        let operations = schema["properties"]["operation"]["enum"]
            .as_array()
            .expect("operation enum should be an array");

        let ops: Vec<&str> = operations.iter().filter_map(|v| v.as_str()).collect();

        assert!(
            ops.contains(&"worktree_add"),
            "schema must include worktree_add"
        );
        assert!(
            ops.contains(&"worktree_remove"),
            "schema must include worktree_remove"
        );
        assert!(
            ops.contains(&"worktree_list"),
            "schema must include worktree_list"
        );
    }

    #[test]
    fn test_schema_contains_worktree_properties() {
        let tool = GitOperationsTool::new();
        let schema = tool.input_schema();
        let properties = schema["properties"]
            .as_object()
            .expect("properties should be an object");

        assert!(
            properties.contains_key("base_ref"),
            "schema must include base_ref property"
        );
        assert!(
            properties.contains_key("force"),
            "schema must include force property"
        );
    }

    /// 辅助: 在临时目录初始化一个可提交的 git 仓库（配置 user.name/email）。
    fn init_test_repo() -> tempfile::TempDir {
        let tmp = tempfile::TempDir::new().unwrap();
        let run = |args: &[&str]| {
            let status = std::process::Command::new("git")
                .args(args)
                .current_dir(tmp.path())
                .output()
                .unwrap();
            assert!(status.status.success(), "git {:?} failed", args);
        };
        run(&["init"]);
        run(&["config", "user.name", "test"]);
        run(&["config", "user.email", "test@test.com"]);
        tmp
    }

    /// 成功路径: status 在已初始化仓库应返回 Ok。
    #[tokio::test]
    async fn git_status_succeeds_in_init_repo() {
        let tmp = init_test_repo();
        let tool = GitOperationsTool::new();
        tool.execute(serde_json::json!({
            "operation": "status",
            "path": tmp.path().to_string_lossy(),
        }))
        .await
        .expect("git status 在已初始化仓库应成功");
    }

    /// 改进 1 验证: 非零退出码返回 Err，且 message 含 stdout/stderr。
    /// 空仓库执行 git log -> 退出码 128 -> Err(git_command_failed)。
    #[tokio::test]
    async fn git_nonzero_exit_returns_error_with_output() {
        let tmp = init_test_repo();
        let tool = GitOperationsTool::new();
        let err = tool
            .execute(serde_json::json!({
                "operation": "log",
                "path": tmp.path().to_string_lossy(),
            }))
            .await
            .unwrap_err();
        assert_eq!(err.code.as_deref(), Some("git_command_failed"));
        assert!(
            err.message.contains("Git command failed"),
            "got: {}",
            err.message
        );
        assert!(
            err.message.contains("stderr"),
            "错误信息应含 stderr 段: {}",
            err.message
        );
    }

    /// 端到端: add -> commit -> log 流程应全部成功。
    #[tokio::test]
    async fn git_add_commit_log_flow() {
        let tmp = init_test_repo();
        std::fs::write(tmp.path().join("a.txt"), "hello").unwrap();
        let tool = GitOperationsTool::new();
        tool.execute(serde_json::json!({
            "operation": "add",
            "path": tmp.path().to_string_lossy(),
            "files": ["a.txt"],
        }))
        .await
        .expect("git add 应成功");
        tool.execute(serde_json::json!({
            "operation": "commit",
            "path": tmp.path().to_string_lossy(),
            "message": "initial commit",
        }))
        .await
        .expect("git commit 应成功");
        let log = tool
            .execute(serde_json::json!({
                "operation": "log",
                "path": tmp.path().to_string_lossy(),
            }))
            .await
            .expect("git log 应成功");
        assert!(
            log.content.contains("initial commit"),
            "got: {}",
            log.content
        );
    }
}
