//! Hooks Module -- lifecycle event hooks for tool execution and sessions.
//!
//! Hooks wrap around the agent loop without modifying it.
//! Configured in ~/.wgenty-code/settings.json under "hooks".
//! Supports both CC nested-array format and legacy flat format.

pub mod cc_adapter;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Types of hook events
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "PascalCase")]
pub enum HookEvent {
    PreToolUse,
    PostToolUse,
    SessionStart,
    SessionEnd,
    Notification,
    /// CC-compatible: Triggered when the agent stops/completes
    Stop,
    /// CC-compatible: Triggered before user prompt is submitted
    UserPromptSubmit,
    /// CC-compatible: Triggered for permission requests
    PermissionRequest,
    /// Triggered when a slash command (e.g. /comet-design) is invoked
    SlashCommand,
}

impl std::fmt::Display for HookEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HookEvent::PreToolUse => write!(f, "PreToolUse"),
            HookEvent::PostToolUse => write!(f, "PostToolUse"),
            HookEvent::SessionStart => write!(f, "SessionStart"),
            HookEvent::SessionEnd => write!(f, "SessionEnd"),
            HookEvent::Notification => write!(f, "Notification"),
            HookEvent::Stop => write!(f, "Stop"),
            HookEvent::UserPromptSubmit => write!(f, "UserPromptSubmit"),
            HookEvent::PermissionRequest => write!(f, "PermissionRequest"),
            HookEvent::SlashCommand => write!(f, "SlashCommand"),
        }
    }
}

/// Source for injected context
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ContextSource {
    Template(String),
    File(PathBuf),
    Inline(String),
}

/// Visibility of an injected context layer
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LayerVisibility {
    Internal,
    Visible,
}

/// An option presented to the user in AskUser action
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserOption {
    pub label: String,
    pub value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Actions that a hook can perform
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HookAction {
    Command {
        command: String,
        timeout_secs: u64,
    },
    InjectContext {
        source: ContextSource,
        priority: u8,
        visibility: LayerVisibility,
    },
    AskUser {
        question: String,
        options: Vec<UserOption>,
    },
}

/// A single hook definition from settings.json
#[derive(Debug, Clone, Serialize)]
pub struct HookDefinition {
    /// The event that triggers this hook
    pub event: HookEvent,
    /// CC-compatible: matcher for filtering hook execution.
    /// None/"" = match all, "ToolA|ToolB" = pipe-separated tool names.
    #[serde(default)]
    pub matcher: Option<String>,
    /// Optional workflow state condition (e.g. "build", "design").
    /// Hook only fires when Comet is in this state, if set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when_state: Option<String>,
    /// Actions to execute when the hook fires
    pub actions: Vec<HookAction>,
}

impl<'de> Deserialize<'de> for HookDefinition {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct HookDefHelper {
            event: HookEvent,
            #[serde(default)]
            matcher: Option<String>,
            #[serde(default)]
            when_state: Option<String>,
            #[serde(default)]
            command: Option<String>,
            #[serde(default)]
            timeout_secs: Option<u64>,
            #[serde(default)]
            #[allow(dead_code)]
            hook_type: Option<String>,
            #[serde(default)]
            actions: Option<Vec<HookAction>>,
        }
        let helper = HookDefHelper::deserialize(deserializer)?;
        let actions = match helper.actions {
            Some(a) if !a.is_empty() => a,
            _ => vec![HookAction::Command {
                command: helper.command.unwrap_or_default(),
                timeout_secs: helper.timeout_secs.unwrap_or(30),
            }],
        };
        Ok(HookDefinition {
            event: helper.event,
            matcher: helper.matcher,
            when_state: helper.when_state,
            actions,
        })
    }
}

/// Check if a hook's matcher matches the given tool name or event.
///
/// - `None` / `""` → matches all
/// - `"ToolA|ToolB"` → matches if tool_name equals any pipe-separated part
/// - For Notification events, the matcher is compared against a notification subtype
pub fn matches_matcher(
    matcher: &Option<String>,
    event: &HookEvent,
    tool_name: Option<&str>,
    notification_subtype: Option<&str>,
) -> bool {
    let pattern_str = match matcher {
        None => return true,
        Some(s) if s.is_empty() => return true,
        Some(s) => s.as_str(),
    };

    // Pipe-separated: try each part
    for part in pattern_str.split('|') {
        let part = part.trim();
        if part.is_empty() {
            return true;
        }
        if *event == HookEvent::Notification {
            if let Some(sub) = notification_subtype {
                if part == sub {
                    return true;
                }
            }
        } else if let Some(name) = tool_name {
            if part == name {
                return true;
            }
        }
    }
    false
}

/// Expand %tool% and %input% variables in a hook command string.
pub fn expand_hook_variables(
    command: &str,
    tool_name: Option<&str>,
    tool_input: Option<&str>,
) -> String {
    let mut result = command.to_string();
    if let Some(name) = tool_name {
        result = result.replace("%tool%", &shell_escape(name));
    }
    if let Some(input) = tool_input {
        result = result.replace("%input%", &shell_escape(input));
    }
    result
}

/// Shell-escape a string by wrapping in single quotes and escaping internal quotes.
fn shell_escape(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_matches_matcher_empty() {
        assert!(matches_matcher(
            &None,
            &HookEvent::PreToolUse,
            Some("TaskCreate"),
            None
        ));
        assert!(matches_matcher(
            &Some("".into()),
            &HookEvent::PreToolUse,
            Some("TaskCreate"),
            None
        ));
    }

    #[test]
    fn test_matches_matcher_single_tool() {
        let matcher = Some("TaskCreate".to_string());
        assert!(matches_matcher(
            &matcher,
            &HookEvent::PreToolUse,
            Some("TaskCreate"),
            None
        ));
        assert!(!matches_matcher(
            &matcher,
            &HookEvent::PreToolUse,
            Some("TaskUpdate"),
            None
        ));
    }

    #[test]
    fn test_matches_matcher_pipe_separated() {
        let matcher = Some("TaskCreate|TaskUpdate".to_string());
        assert!(matches_matcher(
            &matcher,
            &HookEvent::PreToolUse,
            Some("TaskCreate"),
            None
        ));
        assert!(matches_matcher(
            &matcher,
            &HookEvent::PreToolUse,
            Some("TaskUpdate"),
            None
        ));
        assert!(!matches_matcher(
            &matcher,
            &HookEvent::PreToolUse,
            Some("Read"),
            None
        ));
    }

    #[test]
    fn test_matches_matcher_notification() {
        let matcher = Some("permission_prompt".to_string());
        assert!(matches_matcher(
            &matcher,
            &HookEvent::Notification,
            None,
            Some("permission_prompt")
        ));
        assert!(!matches_matcher(
            &matcher,
            &HookEvent::Notification,
            None,
            Some("other")
        ));
    }

    #[test]
    fn test_expand_hook_variables_tool() {
        let result = expand_hook_variables("echo %tool%", Some("TaskCreate"), None);
        assert!(result.contains("TaskCreate"));
        assert!(!result.contains("%tool%"));
    }

    #[test]
    fn test_expand_hook_variables_input() {
        let result = expand_hook_variables("echo %input%", None, Some(r#"{"key":"value"}"#));
        assert!(result.contains(r#"{"key":"value"}"#));
        assert!(!result.contains("%input%"));
    }

    #[test]
    fn test_shell_escape_single_quotes() {
        let escaped = shell_escape("it's working");
        assert_eq!(escaped, "'it'\\''s working'");
    }

    #[test]
    fn test_deserialize_hookevent_slash_command() {
        // GREEN: SlashCommand variant now exists, deserialization should succeed.
        let result: Result<HookEvent, _> = serde_json::from_str("\"SlashCommand\"");
        assert!(result.is_ok(), "SlashCommand should deserialize as a HookEvent variant");
        assert_eq!(result.unwrap(), HookEvent::SlashCommand);
    }

    #[test]
    fn test_hook_action_inject_context_serde() {
        // GREEN: HookAction::InjectContext should serialize/deserialize correctly.
        let action = HookAction::InjectContext {
            source: ContextSource::Inline("hello".to_string()),
            priority: 10,
            visibility: LayerVisibility::Internal,
        };
        let json = serde_json::to_string(&action).expect("serialize InjectContext");
        let parsed: HookAction = serde_json::from_str(&json).expect("deserialize InjectContext");
        match parsed {
            HookAction::InjectContext { source, priority, visibility } => {
                match source {
                    ContextSource::Inline(s) => assert_eq!(s, "hello"),
                    _ => panic!("expected Inline source"),
                }
                assert_eq!(priority, 10);
                match visibility {
                    LayerVisibility::Internal => {},
                    _ => panic!("expected Internal visibility"),
                }
            }
            _ => panic!("expected InjectContext variant"),
        }
    }

    #[test]
    fn test_hook_definition_new_actions_format() {
        // GREEN: New HookDefinition format with 'actions' should now deserialize.
        let json = serde_json::json!({
            "event": "PreToolUse",
            "matcher": "TaskCreate",
            "when_state": "build",
            "actions": [
                {"Command": {"command": "echo hello", "timeout_secs": 10}}
            ]
        });
        let result: Result<HookDefinition, _> = serde_json::from_value(json);
        assert!(result.is_ok(), "New actions format should deserialize after refactor");
        let def = result.unwrap();
        assert_eq!(def.event, HookEvent::PreToolUse);
        assert_eq!(def.matcher.as_deref(), Some("TaskCreate"));
        assert_eq!(def.when_state.as_deref(), Some("build"));
        assert_eq!(def.actions.len(), 1);
    }

    #[test]
    fn test_hook_definition_backward_compat_command_format() {
        // GREEN: Old format with 'command' field must continue to work after refactor.
        let json = serde_json::json!({
            "event": "PreToolUse",
            "command": "echo hello",
            "timeout_secs": 30
        });
        let result: Result<HookDefinition, _> = serde_json::from_value(json);
        assert!(result.is_ok(), "Old command format should deserialize into new HookDefinition");
        let def = result.unwrap();
        assert_eq!(def.event, HookEvent::PreToolUse);
        assert_eq!(def.actions.len(), 1);
        match &def.actions[0] {
            HookAction::Command { command, timeout_secs } => {
                assert_eq!(command, "echo hello");
                assert_eq!(*timeout_secs, 30);
            }
            _ => panic!("expected Command action"),
        }
    }

    #[test]
    fn test_hook_context_workflow_state_and_variables() {
        // RED: HookContext should serialize workflow_state and variables fields.
        let ctx = HookContext {
            event: "PreToolUse".to_string(),
            tool_name: Some("test".to_string()),
            tool_input: None,
            tool_result: None,
            session_id: None,
            working_directory: "/tmp".to_string(),
            timestamp: "2024-01-01T00:00:00Z".to_string(),
            comet_phase: None,
            workflow_state: Some("build".to_string()),
            variables: {
                let mut m = HashMap::new();
                m.insert("key".to_string(), "value".to_string());
                m
            },
        };
        let json = serde_json::to_value(&ctx).expect("serialize HookContext");
        assert_eq!(json["workflow_state"], "build");
        assert_eq!(json["variables"]["key"], "value");
    }
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
    /// Current Comet workflow phase (open/design/build/verify/archive), if any.
    /// Deprecated: use `workflow_state` instead.
    pub comet_phase: Option<String>,
    /// Generic workflow state (replaces comet_phase for the runtime).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workflow_state: Option<String>,
    /// Key-value variables for hook context (e.g., from slash commands).
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub variables: HashMap<String, String>,
}

impl HookContext {
    /// Set the comet_phase field and return self (builder pattern).
    #[deprecated(note = "Use `with_workflow_state` instead")]
    pub fn with_comet_phase(mut self, phase: Option<String>) -> Self {
        self.workflow_state = phase.clone();
        self.comet_phase = phase;
        self
    }

    /// Set the workflow_state (and comet_phase for backward compat) field and return self.
    pub fn with_workflow_state(mut self, state: Option<String>) -> Self {
        self.workflow_state = state.clone();
        self.comet_phase = state;
        self
    }
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

    /// Fire all hooks for an event. Returns outcomes for each hook.
    /// Hooks are filtered by matcher before execution.
    pub async fn fire(
        &self,
        event: &HookEvent,
        ctx: &HookContext,
        notification_subtype: Option<&str>,
    ) -> Vec<HookOutcome> {
        let defs = match self.hooks.get(event) {
            Some(d) => d.clone(),
            None => return vec![],
        };

        let mut outcomes = Vec::new();

        for def in &defs {
            // Filter: skip hooks whose matcher doesn't match
            if !matches_matcher(
                &def.matcher,
                event,
                ctx.tool_name.as_deref(),
                notification_subtype,
            ) {
                continue;
            }
            let outcome = self.execute_hook(def, ctx).await;
            outcomes.push(outcome);
        }

        outcomes
    }

    async fn execute_hook(&self, def: &HookDefinition, ctx: &HookContext) -> HookOutcome {
        let event_label = format!("{:?}", def.event);
        // Execute the first Command action (if multiple, later tasks can handle sequencing).
        let command_action = def.actions.iter().find_map(|a| match a {
            HookAction::Command { command, timeout_secs } => Some((command.clone(), *timeout_secs)),
            _ => None,
        });

        let (command, timeout_secs) = match command_action {
            Some(c) => c,
            None => {
                return HookOutcome {
                    hook_event: event_label,
                    success: true,
                    output: "No command action in hook".to_string(),
                    blocked: false,
                }
            }
        };

        // Expand %tool% and %input% variables
        let expanded_command = expand_hook_variables(
            &command,
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
                // Write context JSON to stdin
                if let Some(stdin) = child.stdin.as_mut() {
                    use tokio::io::AsyncWriteExt;
                    let _ = stdin.write_all(ctx_json.as_bytes()).await;
                    let _ = stdin.flush().await;
                }
                // Drop stdin to signal EOF before waiting
                drop(child.stdin.take());

                tokio::time::timeout(
                    std::time::Duration::from_secs(timeout_secs),
                    child.wait_with_output(),
                )
                .await
            }
            Err(e) => {
                return HookOutcome {
                    hook_event: command.clone(),
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
                            hook_event: command.clone(),
                            success: parsed.continue_execution,
                            output: parsed.reason.unwrap_or_default(),
                            blocked: !parsed.continue_execution,
                        }
                    } else {
                        HookOutcome {
                            hook_event: command.clone(),
                            success: true,
                            output: stdout,
                            blocked: false,
                        }
                    }
                } else {
                    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                    HookOutcome {
                        hook_event: command.clone(),
                        success: false,
                        output: stderr,
                        blocked: false, // don't block on hook failure
                    }
                }
            }
            Ok(Err(e)) => HookOutcome {
                hook_event: command.clone(),
                success: false,
                output: format!("Hook execution error: {}", e),
                blocked: false,
            },
            Err(_) => HookOutcome {
                hook_event: command.clone(),
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
