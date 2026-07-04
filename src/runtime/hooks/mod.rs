//! Hooks Module -- lifecycle event hooks for tool execution and sessions.
//!
//! Hooks wrap around the agent loop without modifying it.
//! Configured in ~/.wgenty-code/settings.json under "hooks".
//! Supports both CC nested-array format and legacy flat format.

pub mod cc_adapter;
pub mod matching;
pub mod types;

use std::collections::HashMap;

pub use matching::{expand_hook_variables, matches_matcher};
pub use types::*;

/// Internal result from running a shell command hook.
struct ShellCommandResult {
    continue_execution: bool,
    reason: Option<String>,
}

/// Manages registered hooks and their execution
#[derive(Default)]
pub struct HookManager {
    hooks: HashMap<HookEvent, Vec<HookDefinition>>,
}

impl HookManager {
    /// Create a new HookManager from settings hooks configuration.
    /// Supports both CC nested-array format and legacy flat format.
    /// Settings format: { "PostToolUse": [{"command": "...", "timeout_secs": 30}] }
    /// CC format: { "PostToolUse": [[{"type": "command", "command": "..."}]] }
    pub fn from_settings(hooks_config: &serde_json::Value) -> Self {
        // First, try CC format (nested arrays with type/matcher fields)
        let cc_hooks = cc_adapter::adapt_cc_hooks(hooks_config);
        if !cc_hooks.is_empty() {
            return Self { hooks: cc_hooks };
        }

        // Fallback: legacy flat format
        let mut hooks: HashMap<HookEvent, Vec<HookDefinition>> = HashMap::new();

        if let Some(obj) = hooks_config.as_object() {
            for (event_name, definitions) in obj {
                let event = match event_name.as_str() {
                    "PreToolUse" => HookEvent::PreToolUse,
                    "PostToolUse" => HookEvent::PostToolUse,
                    "SessionStart" => HookEvent::SessionStart,
                    "SessionEnd" => HookEvent::SessionEnd,
                    "Notification" => HookEvent::Notification,
                    "Stop" => HookEvent::Stop,
                    "UserPromptSubmit" => HookEvent::UserPromptSubmit,
                    "PermissionRequest" => HookEvent::PermissionRequest,
                    "SlashCommand" => HookEvent::SlashCommand,
                    _ => continue,
                };

                if let Some(arr) = definitions.as_array() {
                    let defs: Vec<HookDefinition> = arr
                        .iter()
                        .filter_map(|d| {
                            // Legacy flat format stores definitions without an explicit
                            // "event" field — inject it from the surrounding map key.
                            let mut obj = d.as_object()?.clone();
                            obj.entry("event")
                                .or_insert_with(|| serde_json::Value::String(event_name.clone()));
                            serde_json::from_value(serde_json::Value::Object(obj)).ok()
                        })
                        .collect();
                    if !defs.is_empty() {
                        hooks.insert(event, defs);
                    }
                }
            }
        }

        Self { hooks }
    }

    /// Check if any hooks are registered for an event
    pub fn has_hooks(&self, event: &HookEvent) -> bool {
        self.hooks
            .get(event)
            .map(|h| !h.is_empty())
            .unwrap_or(false)
    }

    /// Register a batch of workflow hooks (e.g., from Comet phase definitions).
    pub fn register_workflow_hooks(&mut self, hooks: Vec<HookDefinition>) {
        for hook in hooks {
            self.hooks.entry(hook.event.clone()).or_default().push(hook);
        }
    }

    /// Fire all hooks for an event. Returns outcomes for each hook.
    /// Hooks are filtered by matcher and optional workflow state.
    /// Pass `state: None` to skip state filtering (backward-compatible).
    /// Pass `notification_subtype` for Notification event matcher matching.
    pub async fn fire(
        &self,
        event: &HookEvent,
        ctx: &HookContext,
        state: Option<&str>,
        notification_subtype: Option<&str>,
    ) -> Vec<HookOutcome> {
        let defs = self.hooks.get(event).map(Vec::as_slice).unwrap_or(&[]);
        let mut outcomes = Vec::new();
        for def in defs {
            // Filter: skip hooks whose matcher doesn't match
            if !matches_matcher(
                &def.matcher,
                event,
                ctx.tool_name.as_deref(),
                notification_subtype,
            ) {
                continue;
            }
            // Filter: when_state condition
            if let Some(ref when) = def.when_state {
                if let Some(current) = state {
                    let states: Vec<&str> = when.split('|').collect();
                    if !states.contains(&current) {
                        continue;
                    }
                }
            }
            // Execute all actions
            for action in &def.actions {
                let outcome = self.execute_action(def, action, ctx).await;
                outcomes.push(outcome);
            }
        }
        outcomes
    }

    /// Execute a single hook action and return the outcome.
    async fn execute_action(
        &self,
        def: &HookDefinition,
        action: &HookAction,
        ctx: &HookContext,
    ) -> HookOutcome {
        match action {
            HookAction::Command {
                command,
                timeout_secs,
            } => {
                let result = self.run_shell_command(command, *timeout_secs, ctx).await;
                HookOutcome {
                    def: def.clone(),
                    continue_execution: result.continue_execution,
                    reason: result.reason,
                    injected_content: None,
                    user_answer: None,
                    injection_priority: None,
                    injection_visibility: None,
                }
            }
            HookAction::InjectContext {
                source,
                priority,
                visibility,
            } => {
                let content = match source {
                    ContextSource::Template(t) => Some(self.render_template(t, ctx)),
                    ContextSource::File(p) => self.read_file_content(p).await,
                    ContextSource::Inline(s) => Some(s.clone()),
                };
                HookOutcome {
                    def: def.clone(),
                    continue_execution: true,
                    reason: None,
                    injected_content: content,
                    user_answer: None,
                    injection_priority: Some(*priority),
                    injection_visibility: Some(visibility.clone()),
                }
            }
            HookAction::AskUser {
                question: _,
                options: _,
            } => {
                // Placeholder: InteractionService will integrate in Task 6
                HookOutcome {
                    def: def.clone(),
                    continue_execution: true,
                    reason: None,
                    injected_content: None,
                    user_answer: Some(UserAnswer { selected: vec![] }),
                    injection_priority: None,
                    injection_visibility: None,
                }
            }
        }
    }

    /// Run a shell command as a hook action and return the parsed result.
    async fn run_shell_command(
        &self,
        command: &str,
        timeout_secs: u64,
        ctx: &HookContext,
    ) -> ShellCommandResult {
        let expanded_command = expand_hook_variables(
            command,
            ctx.tool_name.as_deref(),
            ctx.tool_input.as_ref().map(|v| v.to_string()).as_deref(),
        );

        let ctx_json = serde_json::to_string(ctx).unwrap_or_default();

        let child = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(&expanded_command)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn();

        let result = match child {
            Ok(mut child) => {
                if let Some(stdin) = child.stdin.as_mut() {
                    use tokio::io::AsyncWriteExt;
                    let _ = stdin.write_all(ctx_json.as_bytes()).await;
                    let _ = stdin.flush().await;
                }
                drop(child.stdin.take());

                tokio::time::timeout(
                    std::time::Duration::from_secs(timeout_secs),
                    child.wait_with_output(),
                )
                .await
            }
            Err(e) => {
                return ShellCommandResult {
                    continue_execution: true,
                    reason: Some(format!("Failed to spawn hook: {}", e)),
                }
            }
        };

        match result {
            Ok(Ok(output)) => {
                if output.status.success() {
                    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                    if let Ok(parsed) = serde_json::from_str::<HookResult>(&stdout) {
                        ShellCommandResult {
                            continue_execution: parsed.continue_execution,
                            reason: parsed.reason,
                        }
                    } else {
                        ShellCommandResult {
                            continue_execution: true,
                            reason: Some(stdout),
                        }
                    }
                } else {
                    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                    // Claude Code compatibility: a PreToolUse hook signals "block" by
                    // exiting with code 2 (stderr carries the human-readable reason).
                    // CC-style hooks (e.g., comet-hook-guard.sh) use this protocol
                    // instead of JSON stdout. Treat exit 2 as a block so those hooks
                    // work under wgenty-code, completing the CC-compat story that
                    // cc_adapter::adapt_cc_hooks started. Other non-zero exits remain
                    // "hook error → proceed" so a crashing hook can't hard-lock tools.
                    let blocked = output.status.code() == Some(2);
                    ShellCommandResult {
                        continue_execution: !blocked,
                        reason: Some(stderr),
                    }
                }
            }
            Ok(Err(e)) => ShellCommandResult {
                continue_execution: true,
                reason: Some(format!("Hook execution error: {}", e)),
            },
            Err(_) => ShellCommandResult {
                continue_execution: true,
                reason: Some("Hook timed out".to_string()),
            },
        }
    }

    /// Render a template string by substituting context variables.
    fn render_template(&self, template: &str, ctx: &HookContext) -> String {
        let mut result = template.to_string();
        result = result.replace("{event}", &ctx.event);
        if let Some(ref name) = ctx.tool_name {
            result = result.replace("{tool_name}", name);
        }
        if let Some(ref input) = ctx.tool_input {
            result = result.replace("{tool_input}", &input.to_string());
        }
        if let Some(ref result_text) = ctx.tool_result {
            result = result.replace("{tool_result}", result_text);
        }
        result = result.replace("{working_directory}", &ctx.working_directory);
        result = result.replace("{timestamp}", &ctx.timestamp);
        if let Some(ref phase) = ctx.workflow_state {
            result = result.replace("{workflow_state}", phase);
        }
        result
    }

    /// Read the content of a file (async).
    async fn read_file_content(&self, path: &std::path::Path) -> Option<String> {
        tokio::fs::read_to_string(path).await.ok()
    }

    /// List registered hook events
    pub fn registered_events(&self) -> Vec<HookEvent> {
        self.hooks.keys().cloned().collect()
    }

    // ── Context builders ─────────────────────────────────────────────────

    /// Build a PreToolUse context
    pub fn pre_tool_context(
        tool_name: &str,
        tool_input: &serde_json::Value,
        session_id: Option<&str>,
    ) -> HookContext {
        HookContext {
            event: "PreToolUse".to_string(),
            tool_name: Some(tool_name.to_string()),
            tool_input: Some(tool_input.clone()),
            tool_result: None,
            session_id: session_id.map(|s| s.to_string()),
            working_directory: std::env::current_dir()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            comet_phase: None,
            workflow_state: None,
            variables: HashMap::new(),
        }
    }

    /// Build a PostToolUse context
    pub fn post_tool_context(
        tool_name: &str,
        tool_input: &serde_json::Value,
        tool_result: &str,
        session_id: Option<&str>,
    ) -> HookContext {
        HookContext {
            event: "PostToolUse".to_string(),
            tool_name: Some(tool_name.to_string()),
            tool_input: Some(tool_input.clone()),
            tool_result: Some(tool_result.to_string()),
            session_id: session_id.map(|s| s.to_string()),
            working_directory: std::env::current_dir()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            comet_phase: None,
            workflow_state: None,
            variables: HashMap::new(),
        }
    }

    /// Build a SessionStart context
    pub fn session_start_context(session_id: &str) -> HookContext {
        HookContext {
            event: "SessionStart".to_string(),
            tool_name: None,
            tool_input: None,
            tool_result: None,
            session_id: Some(session_id.to_string()),
            working_directory: std::env::current_dir()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            comet_phase: None,
            workflow_state: None,
            variables: HashMap::new(),
        }
    }

    /// Build a SessionEnd context
    pub fn session_end_context(session_id: &str) -> HookContext {
        HookContext {
            event: "SessionEnd".to_string(),
            tool_name: None,
            tool_input: None,
            tool_result: None,
            session_id: Some(session_id.to_string()),
            working_directory: std::env::current_dir()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            comet_phase: None,
            workflow_state: None,
            variables: HashMap::new(),
        }
    }

    /// Build a Notification context (for CC-compatible notification hooks).
    /// `message` is placed in `tool_input` as a JSON string value.
    pub fn notification_context(message: Option<&str>, session_id: Option<&str>) -> HookContext {
        HookContext {
            event: "Notification".to_string(),
            tool_name: None,
            tool_input: message.map(|m| serde_json::Value::String(m.to_string())),
            tool_result: None,
            session_id: session_id.map(|s| s.to_string()),
            working_directory: std::env::current_dir()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            comet_phase: None,
            workflow_state: None,
            variables: HashMap::new(),
        }
    }
}

#[cfg(test)]
mod tests;
