//! Background task management — run shell commands in the background
//! and receive their results via a notification queue that gets injected
//! into the agent loop on the next iteration.
//!
//! Spawns go through the same mode-linked sandbox policy as foreground
//! shell tools (HardFail vs DegradeWithMark).

use crate::agent::ToolContext;
use crate::sandbox::{shell_command_captured, EffectiveMode, SandboxManager, SandboxOutput};
use crate::tools::execution::sandbox_exec::{
    resolve_for_context, sandbox_infra_tool_error, sandbox_metadata, should_degrade_to_direct,
};
use crate::tools::{Tool, ToolError, ToolOutput};
use async_trait::async_trait;
use serde::Serialize;
use std::sync::Arc;
use tokio::sync::Mutex;

/// A completed background task result
#[derive(Debug, Clone, Serialize)]
pub struct BackgroundResult {
    pub task_id: String,
    pub result_type: String,
    pub command: String,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<i32>,
    pub success: bool,
    /// True when the command ran outside the OS sandbox (degrade / disabled).
    #[serde(default)]
    pub sandbox_bypassed: bool,
    #[serde(default)]
    pub permission_mode: Option<String>,
    #[serde(default)]
    pub sandbox_level: Option<String>,
}

/// Manages background command execution and notification delivery.
pub struct BackgroundManager {
    results: Arc<Mutex<Vec<BackgroundResult>>>,
    next_id: Arc<Mutex<u64>>,
    /// Shared with shell tools when attached via [`with_sandbox`](Self::with_sandbox).
    pub(crate) sandbox: Option<Arc<SandboxManager>>,
}

impl BackgroundManager {
    pub fn new() -> Self {
        Self {
            results: Arc::new(Mutex::new(Vec::new())),
            next_id: Arc::new(Mutex::new(1)),
            sandbox: None,
        }
    }

    /// Attach a sandbox manager so background spawns share shell isolation.
    pub fn with_sandbox(mut self, sandbox: Arc<SandboxManager>) -> Self {
        self.sandbox = Some(sandbox);
        self
    }

    /// Spawn a command in the background under the mode-linked sandbox policy.
    ///
    /// Returns the task_id immediately. Infrastructure HardFail is returned
    /// before the task is queued so callers never believe a denied spawn started.
    pub async fn spawn(
        &self,
        command: &str,
        timeout_secs: u64,
        mode: EffectiveMode,
        workdir: Option<&std::path::Path>,
    ) -> Result<String, ToolError> {
        let mut id_lock = self.next_id.lock().await;
        let task_id = format!("bg_{}", *id_lock);
        *id_lock += 1;
        drop(id_lock);

        let mut policy = resolve_for_context(mode, workdir, None);
        policy.profile.resources.max_wall_seconds = timeout_secs.max(300);

        let backend_name = self
            .sandbox
            .as_ref()
            .map(|sb| sb.status().backend_name.clone())
            .unwrap_or_else(|| "none".to_string());
        let hardware_enforced = self
            .sandbox
            .as_ref()
            .map(|sb| sb.status().is_hardware_enforced)
            .unwrap_or(false);

        // Resolve whether we will attempt sandbox or must degrade / hard-fail now.
        let mut start_bypassed = !policy.enabled;
        if policy.enabled && self.sandbox.is_none() {
            if !should_degrade_to_direct(policy.fail_mode) {
                return Err(sandbox_infra_tool_error(
                    "none",
                    "sandbox manager not attached",
                ));
            }
            start_bypassed = true;
        }

        let task_id_clone = task_id.clone();
        let command_clone = command.to_string();
        let results = self.results.clone();
        let sandbox = self.sandbox.clone();
        let profile = policy.profile.clone();
        let fail_mode = policy.fail_mode;
        let level = policy.level;
        let enabled = policy.enabled;
        let mode_str = mode.as_str().to_string();
        let level_str = level.as_str().to_string();
        let workdir_owned = workdir.map(|p| p.to_path_buf());

        tokio::spawn(async move {
            let timeout = std::time::Duration::from_secs(timeout_secs.max(300));
            let cmd_inner = command_clone.clone();

            let run = async {
                // Prefer sandboxed execute when enabled and manager present.
                if enabled {
                    if let Some(ref sb) = sandbox {
                        match sb.execute(&cmd_inner, &profile).await {
                            Ok(out) => {
                                return background_from_sandbox(
                                    &task_id_clone,
                                    &cmd_inner,
                                    out,
                                    false,
                                    &mode_str,
                                    &level_str,
                                );
                            }
                            Err(e) => {
                                if !should_degrade_to_direct(fail_mode) {
                                    return BackgroundResult {
                                        task_id: task_id_clone.clone(),
                                        result_type: "command".to_string(),
                                        command: cmd_inner.clone(),
                                        stdout: String::new(),
                                        stderr: format!(
                                            "sandbox unavailable ({}): {}",
                                            sb.status().backend_name,
                                            e
                                        ),
                                        exit_code: None,
                                        success: false,
                                        sandbox_bypassed: false,
                                        permission_mode: Some(mode_str.clone()),
                                        sandbox_level: Some(level_str.clone()),
                                    };
                                }
                                tracing::warn!(
                                    command = %cmd_inner,
                                    backend = %sb.status().backend_name,
                                    error = %e,
                                    "Background sandbox failed; degrading to direct"
                                );
                            }
                        }
                    }
                }

                // Direct (disabled, degrade, or missing manager under degrade).
                let mut cmd = shell_command_captured(&cmd_inner);
                if let Some(ref dir) = workdir_owned {
                    cmd.current_dir(dir);
                }
                match cmd.output().await {
                    Ok(out) => BackgroundResult {
                        task_id: task_id_clone.clone(),
                        result_type: "command".to_string(),
                        command: cmd_inner.clone(),
                        stdout: String::from_utf8_lossy(&out.stdout).to_string(),
                        stderr: String::from_utf8_lossy(&out.stderr).to_string(),
                        exit_code: out.status.code(),
                        success: out.status.success(),
                        sandbox_bypassed: true,
                        permission_mode: Some(mode_str.clone()),
                        sandbox_level: Some(level_str.clone()),
                    },
                    Err(e) => BackgroundResult {
                        task_id: task_id_clone.clone(),
                        result_type: "command".to_string(),
                        command: cmd_inner,
                        stdout: String::new(),
                        stderr: format!("Failed to execute: {}", e),
                        exit_code: None,
                        success: false,
                        sandbox_bypassed: true,
                        permission_mode: Some(mode_str.clone()),
                        sandbox_level: Some(level_str.clone()),
                    },
                }
            };

            // Silence unused warning when start_bypassed only used for future metadata.
            let _ = (start_bypassed, hardware_enforced, backend_name);

            let final_result = match tokio::time::timeout(timeout, run).await {
                Ok(r) => r,
                Err(_) => BackgroundResult {
                    task_id: task_id_clone,
                    result_type: "command".to_string(),
                    command: command_clone,
                    stdout: String::new(),
                    stderr: "Command timed out".to_string(),
                    exit_code: None,
                    success: false,
                    sandbox_bypassed: start_bypassed,
                    permission_mode: Some(mode_str),
                    sandbox_level: Some(level_str),
                },
            };

            let mut results = results.lock().await;
            results.push(final_result);
        });

        Ok(task_id)
    }

    /// Push a subagent result into the completed queue (for background subagents).
    pub async fn push_subagent_result(&self, description: &str, stdout: &str, success: bool) {
        let mut id_lock = self.next_id.lock().await;
        let task_id = format!("subagent_{}", *id_lock);
        *id_lock += 1;
        drop(id_lock);

        let mut results = self.results.lock().await;
        results.push(BackgroundResult {
            task_id,
            result_type: "subagent".to_string(),
            command: description.to_string(),
            stdout: stdout.to_string(),
            stderr: String::new(),
            exit_code: if success { Some(0) } else { Some(1) },
            success,
            sandbox_bypassed: false,
            permission_mode: None,
            sandbox_level: None,
        });
    }

    /// Drain all completed results from the queue.
    pub async fn drain_results(&self) -> Vec<BackgroundResult> {
        let mut results = self.results.lock().await;
        std::mem::take(&mut *results)
    }

    /// Check if any results are ready.
    pub async fn has_results(&self) -> bool {
        let results = self.results.lock().await;
        !results.is_empty()
    }

    /// Format results as a notification message for injection into the agent loop.
    pub fn format_notification(results: &[BackgroundResult]) -> Option<String> {
        if results.is_empty() {
            return None;
        }

        let mut lines = vec!["## Background Task Results\n".to_string()];
        for r in results {
            let status = if r.success { "SUCCESS" } else { "FAILED" };
            let output = if r.success { &r.stdout } else { &r.stderr };
            let bypass = if r.sandbox_bypassed {
                " [sandbox bypassed]"
            } else {
                ""
            };
            let truncated = if output.chars().count() > 2000 {
                let t: String = output.chars().take(2000).collect();
                format!("{}…", t)
            } else {
                output.clone()
            };
            lines.push(format!(
                "### {} [{}]{} (exit: {:?})\n```\n{}\n```",
                r.task_id, status, bypass, r.exit_code, truncated
            ));
        }
        Some(lines.join("\n"))
    }
}

fn background_from_sandbox(
    task_id: &str,
    command: &str,
    out: SandboxOutput,
    bypassed: bool,
    mode: &str,
    level: &str,
) -> BackgroundResult {
    let success = out.exit_code == 0 && !out.killed_by_sandbox;
    let stderr = if out.killed_by_sandbox {
        format!("killed by sandbox\n{}", out.stderr)
    } else {
        out.stderr
    };
    BackgroundResult {
        task_id: task_id.to_string(),
        result_type: "command".to_string(),
        command: command.to_string(),
        stdout: out.stdout,
        stderr,
        exit_code: Some(out.exit_code),
        success,
        sandbox_bypassed: bypassed,
        permission_mode: Some(mode.to_string()),
        sandbox_level: Some(level.to_string()),
    }
}

impl Default for BackgroundManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Tool for running shell commands in the background.
pub struct BackgroundTool {
    manager: Arc<BackgroundManager>,
}

impl BackgroundTool {
    pub fn new(manager: Arc<BackgroundManager>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl Tool for BackgroundTool {
    fn name(&self) -> &str {
        "background"
    }

    fn description(&self) -> &str {
        "Run a shell command in the background. Returns a task_id immediately. \
         Results will be delivered when the command completes. \
         Use for long-running commands where you don't need the result right away. \
         Uses the same sandbox policy as execute_command for the current permission mode."
    }

    fn is_read_only(&self) -> bool {
        false
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The shell command to run in the background"
                },
                "timeout_secs": {
                    "type": "integer",
                    "description": "Max execution time in seconds (default 300)",
                    "default": 300
                }
            },
            "required": ["command"]
        })
    }

    async fn execute(&self, input: serde_json::Value) -> Result<ToolOutput, ToolError> {
        self.run(input, EffectiveMode::default(), None).await
    }

    async fn execute_with_context(
        &self,
        context: &ToolContext<'_>,
        input: serde_json::Value,
    ) -> Result<ToolOutput, ToolError> {
        self.run(input, context.effective_mode, context.workdir)
            .await
    }
}

impl BackgroundTool {
    async fn run(
        &self,
        input: serde_json::Value,
        mode: EffectiveMode,
        workdir: Option<&std::path::Path>,
    ) -> Result<ToolOutput, ToolError> {
        let command = input["command"].as_str().unwrap_or("");
        if command.is_empty() {
            return Err(ToolError {
                message: "command is required".to_string(),
                code: Some("missing_parameter".to_string()),
            });
        }
        let timeout = input["timeout_secs"].as_u64().unwrap_or(300);

        let policy = resolve_for_context(mode, workdir, None);
        let backend_name = self
            .manager
            .sandbox
            .as_ref()
            .map(|sb| sb.status().backend_name.clone())
            .unwrap_or_else(|| "none".to_string());
        let hardware = self
            .manager
            .sandbox
            .as_ref()
            .map(|sb| sb.status().is_hardware_enforced)
            .unwrap_or(false);

        // Pre-check HardFail when no sandbox manager and enabled (spawn also checks).
        let task_id = self.manager.spawn(command, timeout, mode, workdir).await?;

        // At accept time we do not yet know if runtime will degrade; mark intent.
        let bypassed = !policy.enabled
            || self.manager.sandbox.is_none() && should_degrade_to_direct(policy.fail_mode);
        let metadata = sandbox_metadata(
            mode,
            policy.level,
            &backend_name,
            bypassed,
            hardware && !bypassed,
            policy.fail_mode,
        );

        Ok(ToolOutput {
            output_type: "json".to_string(),
            content: serde_json::json!({
                "success": true,
                "task_id": task_id,
                "message": format!(
                    "Background task '{}' started. Results will be delivered when ready.",
                    task_id
                )
            })
            .to_string(),
            metadata,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{AgentExecutionContext, SessionId, ToolInvocationId};
    use serde_json::json;

    #[tokio::test]
    async fn normal_without_sandbox_hard_fails() {
        let mgr = Arc::new(BackgroundManager::new());
        let tool = BackgroundTool::new(mgr);
        let err = tool
            .execute(json!({"command": "echo hi"}))
            .await
            .unwrap_err();
        assert_eq!(err.code.as_deref(), Some("sandbox_spawn_failed"));
    }

    #[tokio::test]
    async fn yolo_without_sandbox_starts() {
        let mgr = Arc::new(BackgroundManager::new());
        let tool = BackgroundTool::new(mgr);
        let root = AgentExecutionContext::root(SessionId::new("test-session"));
        let ctx = ToolContext {
            agent: &root,
            invocation_id: ToolInvocationId::new("inv"),
            origin_turn_id: None,
            workdir: None,
            effective_mode: EffectiveMode::Yolo,
            checkpoint: None,
        };
        let out = tool
            .execute_with_context(&ctx, json!({"command": "echo hi"}))
            .await
            .expect("yolo should allow degrade");
        assert_eq!(
            out.metadata
                .get("sandbox_bypassed")
                .and_then(|v| v.as_bool()),
            Some(true)
        );
    }
}
