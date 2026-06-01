//! Hooks Module -- lifecycle event hooks for tool execution and sessions.
//!
//! Hooks wrap around the agent loop without modifying it.
//! Configured in ~/.wgenty-code/settings.json under "hooks".

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Types of hook events
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum HookEvent {
    PreToolUse,
    PostToolUse,
    SessionStart,
    SessionEnd,
    Notification,
}

impl std::fmt::Display for HookEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HookEvent::PreToolUse => write!(f, "PreToolUse"),
            HookEvent::PostToolUse => write!(f, "PostToolUse"),
            HookEvent::SessionStart => write!(f, "SessionStart"),
            HookEvent::SessionEnd => write!(f, "SessionEnd"),
            HookEvent::Notification => write!(f, "Notification"),
        }
    }
}

/// A single hook definition from settings.json
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookDefinition {
    /// Shell command to execute
    pub command: String,
    /// Optional timeout in seconds (default 30)
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
}

fn default_timeout() -> u64 {
    30
}

/// Context passed to hooks via stdin (JSON)
#[derive(Debug, Clone, Serialize)]
pub struct HookContext {
    pub event: String,
    pub tool_name: Option<String>,
    pub tool_input: Option<serde_json::Value>,
    pub tool_result: Option<String>,
    pub session_id: Option<String>,
    pub working_directory: String,
    pub timestamp: String,
}

/// Result returned from a hook via stdout (JSON)
#[derive(Debug, Clone, Deserialize)]
pub struct HookResult {
    #[serde(default)]
    pub continue_execution: bool, // true = proceed, false = block
    #[serde(default)]
    pub reason: Option<String>, // reason if blocked
}

/// Outcome of executing a single hook
#[derive(Debug, Clone)]
pub struct HookOutcome {
    pub hook_event: String,
    pub success: bool,
    pub output: String,
    pub blocked: bool,
}

/// Manages registered hooks and their execution
pub struct HookManager {
    hooks: HashMap<HookEvent, Vec<HookDefinition>>,
}

impl HookManager {
    /// Create a new HookManager from settings hooks configuration.
    /// Settings format: { "hooks": { "PreToolUse": [{"command": "...", "timeout_secs": 30}] } }
    pub fn from_settings(hooks_config: &serde_json::Value) -> Self {
        let mut hooks: HashMap<HookEvent, Vec<HookDefinition>> = HashMap::new();

        if let Some(obj) = hooks_config.as_object() {
            for (event_name, definitions) in obj {
                let event = match event_name.as_str() {
                    "PreToolUse" => HookEvent::PreToolUse,
                    "PostToolUse" => HookEvent::PostToolUse,
                    "SessionStart" => HookEvent::SessionStart,
                    "SessionEnd" => HookEvent::SessionEnd,
                    "Notification" => HookEvent::Notification,
                    _ => continue,
                };

                if let Some(arr) = definitions.as_array() {
                    let defs: Vec<HookDefinition> = arr
                        .iter()
                        .filter_map(|d| serde_json::from_value(d.clone()).ok())
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

    /// Fire all hooks for an event. Returns outcomes for each hook.
    pub async fn fire(&self, event: &HookEvent, ctx: &HookContext) -> Vec<HookOutcome> {
        let defs = match self.hooks.get(event) {
            Some(d) => d.clone(),
            None => return vec![],
        };

        let mut outcomes = Vec::new();

        for def in &defs {
            let outcome = self.execute_hook(def, ctx).await;
            outcomes.push(outcome);
        }

        outcomes
    }

    async fn execute_hook(&self, def: &HookDefinition, ctx: &HookContext) -> HookOutcome {
        let ctx_json = serde_json::to_string(ctx).unwrap_or_default();

        let child = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(&def.command)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn();

        let result = match child {
            Ok(mut child) => {
                // Write context JSON to stdin
                if let Some(stdin) = child.stdin.as_mut() {
                    use tokio::io::AsyncWriteExt;
                    let _ = stdin.write_all(ctx_json.as_bytes()).await;
                    let _ = stdin.flush().await;
                }
                // Drop stdin to signal EOF before waiting
                drop(child.stdin.take());

                tokio::time::timeout(
                    std::time::Duration::from_secs(def.timeout_secs),
                    child.wait_with_output(),
                )
                .await
            }
            Err(e) => {
                return HookOutcome {
                    hook_event: def.command.clone(),
                    success: false,
                    output: format!("Failed to spawn hook: {}", e),
                    blocked: false,
                }
            }
        };

        match result {
            Ok(Ok(output)) => {
                if output.status.success() {
                    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                    // Try to parse JSON result from stdout
                    if let Ok(parsed) = serde_json::from_str::<HookResult>(&stdout) {
                        HookOutcome {
                            hook_event: def.command.clone(),
                            success: parsed.continue_execution,
                            output: parsed.reason.unwrap_or_default(),
                            blocked: !parsed.continue_execution,
                        }
                    } else {
                        HookOutcome {
                            hook_event: def.command.clone(),
                            success: true,
                            output: stdout,
                            blocked: false,
                        }
                    }
                } else {
                    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                    HookOutcome {
                        hook_event: def.command.clone(),
                        success: false,
                        output: stderr,
                        blocked: false, // don't block on hook failure
                    }
                }
            }
            Ok(Err(e)) => HookOutcome {
                hook_event: def.command.clone(),
                success: false,
                output: format!("Hook execution error: {}", e),
                blocked: false,
            },
            Err(_) => HookOutcome {
                hook_event: def.command.clone(),
                success: false,
                output: "Hook timed out".to_string(),
                blocked: false,
            },
        }
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
        }
    }
}

impl Default for HookManager {
    fn default() -> Self {
        Self {
            hooks: HashMap::new(),
        }
    }
}
