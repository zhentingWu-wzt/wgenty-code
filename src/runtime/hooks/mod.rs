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
        assert!(
            result.is_ok(),
            "SlashCommand should deserialize as a HookEvent variant"
        );
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
            HookAction::InjectContext {
                source,
                priority,
                visibility,
            } => {
                match source {
                    ContextSource::Inline(s) => assert_eq!(s, "hello"),
                    _ => panic!("expected Inline source"),
                }
                assert_eq!(priority, 10);
                match visibility {
                    LayerVisibility::Internal => {}
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
        assert!(
            result.is_ok(),
            "New actions format should deserialize after refactor"
        );
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
        assert!(
            result.is_ok(),
            "Old command format should deserialize into new HookDefinition"
        );
        let def = result.unwrap();
        assert_eq!(def.event, HookEvent::PreToolUse);
        assert_eq!(def.actions.len(), 1);
        match &def.actions[0] {
            HookAction::Command {
                command,
                timeout_secs,
            } => {
                assert_eq!(command, "echo hello");
                assert_eq!(*timeout_secs, 30);
            }
            _ => panic!("expected Command action"),
        }
    }

    #[test]
    fn test_user_answer_struct() {
        // RED: UserAnswer struct should exist with selected field.
        let answer = UserAnswer {
            selected: vec!["opt1".to_string(), "opt2".to_string()],
        };
        assert_eq!(answer.selected.len(), 2);
        assert_eq!(answer.selected[0], "opt1");
        assert_eq!(answer.selected[1], "opt2");
    }

    #[test]
    fn test_hook_outcome_new_fields() {
        // RED: HookOutcome should have def, continue_execution, reason,
        // injected_content, and user_answer fields.
        let def = HookDefinition {
            event: HookEvent::PreToolUse,
            matcher: None,
            when_state: None,
            actions: vec![],
        };
        let outcome = HookOutcome {
            def: def.clone(),
            continue_execution: true,
            reason: Some("test reason".to_string()),
            injected_content: Some("injected text".to_string()),
            user_answer: Some(UserAnswer {
                selected: vec!["a".to_string()],
            }),
            injection_priority: None,
            injection_visibility: None,
        };
        assert_eq!(outcome.continue_execution, true);
        assert_eq!(outcome.reason.as_deref(), Some("test reason"));
        assert_eq!(outcome.injected_content.as_deref(), Some("injected text"));
        assert!(outcome.user_answer.is_some());
        assert_eq!(outcome.user_answer.as_ref().unwrap().selected, vec!["a"]);
        assert_eq!(outcome.def.event, HookEvent::PreToolUse);
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

    // ── Step 3: execute_action tests (via fire()) ──────────────────────

    #[tokio::test]
    async fn test_fire_inject_context_inline() {
        // RED → GREEN: InjectContext with Inline source produces injected_content.
        let mut hm = HookManager::default();
        let def = HookDefinition {
            event: HookEvent::PreToolUse,
            matcher: None,
            when_state: None,
            actions: vec![HookAction::InjectContext {
                source: ContextSource::Inline("hello world".to_string()),
                priority: 10,
                visibility: LayerVisibility::Internal,
            }],
        };
        hm.register_workflow_hooks(vec![def]);
        let ctx = HookManager::pre_tool_context("test", &serde_json::json!({}), None);
        let outcomes = hm.fire(&HookEvent::PreToolUse, &ctx, None, None).await;
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].injected_content.as_deref(), Some("hello world"));
        assert!(outcomes[0].continue_execution);
    }

    #[tokio::test]
    async fn test_fire_inject_context_template() {
        // RED → GREEN: InjectContext with Template source renders variables.
        let mut hm = HookManager::default();
        let def = HookDefinition {
            event: HookEvent::PreToolUse,
            matcher: None,
            when_state: None,
            actions: vec![HookAction::InjectContext {
                source: ContextSource::Template("Tool: {tool_name}".to_string()),
                priority: 10,
                visibility: LayerVisibility::Internal,
            }],
        };
        hm.register_workflow_hooks(vec![def]);
        let ctx = HookManager::pre_tool_context("MyTool", &serde_json::json!({}), None);
        let outcomes = hm.fire(&HookEvent::PreToolUse, &ctx, None, None).await;
        assert_eq!(outcomes.len(), 1);
        assert_eq!(
            outcomes[0].injected_content.as_deref(),
            Some("Tool: MyTool")
        );
    }

    #[tokio::test]
    async fn test_fire_ask_user_placeholder() {
        // RED → GREEN: AskUser returns a UserAnswer with empty selected (placeholder).
        let mut hm = HookManager::default();
        let def = HookDefinition {
            event: HookEvent::PreToolUse,
            matcher: None,
            when_state: None,
            actions: vec![HookAction::AskUser {
                question: "Proceed?".to_string(),
                options: vec![UserOption {
                    label: "Yes".to_string(),
                    value: "yes".to_string(),
                    description: None,
                }],
            }],
        };
        hm.register_workflow_hooks(vec![def]);
        let ctx = HookManager::pre_tool_context("test", &serde_json::json!({}), None);
        let outcomes = hm.fire(&HookEvent::PreToolUse, &ctx, None, None).await;
        assert_eq!(outcomes.len(), 1);
        assert!(outcomes[0].user_answer.is_some());
        assert!(outcomes[0]
            .user_answer
            .as_ref()
            .unwrap()
            .selected
            .is_empty());
        assert!(outcomes[0].continue_execution);
    }

    // ── Step 4: register_workflow_hooks tests ────────────────────────

    #[test]
    fn test_register_workflow_hooks_adds_hooks() {
        // RED → GREEN: register_workflow_hooks makes hooks visible via has_hooks.
        let mut hm = HookManager::default();
        assert!(!hm.has_hooks(&HookEvent::PreToolUse));
        let def = HookDefinition {
            event: HookEvent::PreToolUse,
            matcher: None,
            when_state: None,
            actions: vec![HookAction::Command {
                command: "echo test".to_string(),
                timeout_secs: 30,
            }],
        };
        hm.register_workflow_hooks(vec![def]);
        assert!(hm.has_hooks(&HookEvent::PreToolUse));
    }

    #[test]
    fn test_register_workflow_hooks_multiple_events() {
        // RED → GREEN: register_workflow_hooks supports multiple events.
        let mut hm = HookManager::default();
        let hooks = vec![
            HookDefinition {
                event: HookEvent::PreToolUse,
                matcher: None,
                when_state: None,
                actions: vec![HookAction::Command {
                    command: "echo a".to_string(),
                    timeout_secs: 30,
                }],
            },
            HookDefinition {
                event: HookEvent::PostToolUse,
                matcher: None,
                when_state: None,
                actions: vec![HookAction::Command {
                    command: "echo b".to_string(),
                    timeout_secs: 30,
                }],
            },
        ];
        hm.register_workflow_hooks(hooks);
        assert!(hm.has_hooks(&HookEvent::PreToolUse));
        assert!(hm.has_hooks(&HookEvent::PostToolUse));
        assert!(!hm.has_hooks(&HookEvent::SessionStart));
    }

    // ── Step 2: when_state filtering tests ───────────────────────────

    #[tokio::test]
    async fn test_fire_when_state_filter_matches() {
        // RED → GREEN: fire() with state matches when_state and fires the hook.
        let mut hm = HookManager::default();
        let def = HookDefinition {
            event: HookEvent::PreToolUse,
            matcher: None,
            when_state: Some("build".to_string()),
            actions: vec![HookAction::InjectContext {
                source: ContextSource::Inline("matched".to_string()),
                priority: 10,
                visibility: LayerVisibility::Internal,
            }],
        };
        hm.register_workflow_hooks(vec![def]);
        let ctx = HookManager::pre_tool_context("test", &serde_json::json!({}), None);
        let outcomes = hm
            .fire(&HookEvent::PreToolUse, &ctx, Some("build"), None)
            .await;
        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].injected_content.as_deref(), Some("matched"));
    }

    #[tokio::test]
    async fn test_fire_when_state_filter_skips() {
        // RED → GREEN: fire() with non-matching state skips the hook.
        let mut hm = HookManager::default();
        let def = HookDefinition {
            event: HookEvent::PreToolUse,
            matcher: None,
            when_state: Some("build".to_string()),
            actions: vec![HookAction::InjectContext {
                source: ContextSource::Inline("should not fire".to_string()),
                priority: 10,
                visibility: LayerVisibility::Internal,
            }],
        };
        hm.register_workflow_hooks(vec![def]);
        let ctx = HookManager::pre_tool_context("test", &serde_json::json!({}), None);
        let outcomes = hm
            .fire(&HookEvent::PreToolUse, &ctx, Some("design"), None)
            .await;
        assert!(outcomes.is_empty());
    }

    #[tokio::test]
    async fn test_fire_when_state_none_fires_all() {
        // RED → GREEN: fire() with state=None fires hooks regardless of when_state.
        let mut hm = HookManager::default();
        let def = HookDefinition {
            event: HookEvent::PreToolUse,
            matcher: None,
            when_state: Some("build".to_string()),
            actions: vec![HookAction::InjectContext {
                source: ContextSource::Inline("always fires".to_string()),
                priority: 10,
                visibility: LayerVisibility::Internal,
            }],
        };
        hm.register_workflow_hooks(vec![def]);
        let ctx = HookManager::pre_tool_context("test", &serde_json::json!({}), None);
        let outcomes = hm.fire(&HookEvent::PreToolUse, &ctx, None, None).await;
        assert_eq!(outcomes.len(), 1);
        assert_eq!(
            outcomes[0].injected_content.as_deref(),
            Some("always fires")
        );
    }

    #[tokio::test]
    async fn test_fire_when_state_pipe_separated() {
        // RED → GREEN: when_state with pipe-separated values matches any.
        let mut hm = HookManager::default();
        let def = HookDefinition {
            event: HookEvent::PreToolUse,
            matcher: None,
            when_state: Some("build|design".to_string()),
            actions: vec![HookAction::InjectContext {
                source: ContextSource::Inline("pipe matched".to_string()),
                priority: 10,
                visibility: LayerVisibility::Internal,
            }],
        };
        hm.register_workflow_hooks(vec![def]);
        let ctx = HookManager::pre_tool_context("test", &serde_json::json!({}), None);
        let outcomes = hm
            .fire(&HookEvent::PreToolUse, &ctx, Some("design"), None)
            .await;
        assert_eq!(outcomes.len(), 1);
        assert_eq!(
            outcomes[0].injected_content.as_deref(),
            Some("pipe matched")
        );
    }

    // ── Step 5: with_state builder test ──────────────────────────────

    #[test]
    fn test_hook_context_with_state_builder() {
        // RED → GREEN: with_state sets workflow_state and comet_phase.
        let ctx = HookManager::pre_tool_context("test", &serde_json::json!({}), None)
            .with_state(Some("build".to_string()));
        assert_eq!(ctx.workflow_state.as_deref(), Some("build"));
        assert_eq!(ctx.comet_phase.as_deref(), Some("build"));
    }

    // ── Notification matcher integration test ─────────────────────────

    #[tokio::test]
    async fn test_fire_notification_matcher() {
        // RED: Notification hook with matcher should work through fire().
        let mut hm = HookManager::default();
        let def = HookDefinition {
            event: HookEvent::Notification,
            matcher: Some("test_subtype".to_string()),
            when_state: None,
            actions: vec![HookAction::InjectContext {
                source: ContextSource::Inline("notification fired".to_string()),
                priority: 10,
                visibility: LayerVisibility::Internal,
            }],
        };
        hm.register_workflow_hooks(vec![def]);
        let ctx = HookManager::notification_context(Some("test message"), None);

        // With matching notification_subtype, hook should fire
        let outcomes = hm
            .fire(&HookEvent::Notification, &ctx, None, Some("test_subtype"))
            .await;
        assert_eq!(outcomes.len(), 1);
        assert_eq!(
            outcomes[0].injected_content.as_deref(),
            Some("notification fired")
        );

        // With non-matching notification_subtype, hook should not fire
        let outcomes = hm
            .fire(&HookEvent::Notification, &ctx, None, Some("other_subtype"))
            .await;
        assert!(outcomes.is_empty());
    }

    #[test]
    fn collect_injections_empty_outcomes_returns_empty() {
        assert!(collect_injections(&[]).is_empty());
    }

    #[test]
    fn collect_injections_single_outcome_extracts_fragment() {
        let outcome = HookOutcome {
            def: HookDefinition {
                event: HookEvent::UserPromptSubmit,
                matcher: None,
                when_state: None,
                actions: vec![],
            },
            continue_execution: true,
            reason: None,
            injected_content: Some("hello".into()),
            user_answer: None,
            injection_priority: Some(20),
            injection_visibility: Some(LayerVisibility::Internal),
        };
        let frags = collect_injections(&[outcome]);
        assert_eq!(frags.len(), 1);
        assert_eq!(frags[0].content, "hello");
        assert_eq!(frags[0].priority, 20);
        assert_eq!(frags[0].source_label, "hook:UserPromptSubmit:0");
        matches!(frags[0].visibility, LayerVisibility::Internal);
    }

    #[test]
    fn collect_injections_sorts_by_priority_stable() {
        let mk = |content: &str, prio: u8| HookOutcome {
            def: HookDefinition {
                event: HookEvent::UserPromptSubmit,
                matcher: None,
                when_state: None,
                actions: vec![],
            },
            continue_execution: true,
            reason: None,
            injected_content: Some(content.into()),
            user_answer: None,
            injection_priority: Some(prio),
            injection_visibility: Some(LayerVisibility::Visible),
        };
        let outcomes = vec![mk("low2", 30), mk("high", 10), mk("low1", 30)];
        let frags = collect_injections(&outcomes);
        assert_eq!(
            frags.iter().map(|f| f.content.as_str()).collect::<Vec<_>>(),
            vec!["high", "low2", "low1"]
        );
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

    /// Set the workflow_state field and return self (shorthand alias).
    pub fn with_state(mut self, state: Option<String>) -> Self {
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

/// User's answer to an AskUser action
#[derive(Debug, Clone)]
pub struct UserAnswer {
    pub selected: Vec<String>,
}

/// Outcome of executing a single hook
#[derive(Debug, Clone)]
pub struct HookOutcome {
    pub def: HookDefinition,
    pub continue_execution: bool,
    pub reason: Option<String>,
    pub injected_content: Option<String>,
    pub user_answer: Option<UserAnswer>,
    // 新增：当 outcome 来自 InjectContext 时填充
    pub injection_priority: Option<u8>,
    pub injection_visibility: Option<LayerVisibility>,
}

/// A normalized injection fragment derived from one or more `HookOutcome`s.
/// Consumers (e.g. the `<system-reminder>` channel) collect these via
/// [`collect_injections`] and render them in priority order.
#[derive(Debug, Clone)]
pub struct InjectedFragment {
    pub content: String,
    pub priority: u8,
    pub visibility: LayerVisibility,
    pub source_label: String,
}

/// Collect `InjectedFragment`s from a slice of `HookOutcome`s.
///
/// - Outcomes without `injected_content` (or with empty content) are skipped.
/// - Priority defaults to `50` and visibility to `Visible` when not provided.
/// - Fragments are sorted by `priority` ascending. The sort is stable, so
///   ties preserve the original outcome order.
/// - `source_label` is `"hook:UserPromptSubmit:<idx>"` where `idx` is the
///   zero-based index of the outcome in the input slice.
pub fn collect_injections(outcomes: &[HookOutcome]) -> Vec<InjectedFragment> {
    let mut out: Vec<InjectedFragment> = outcomes
        .iter()
        .enumerate()
        .filter_map(|(idx, oc)| {
            let content = oc.injected_content.as_ref()?;
            if content.is_empty() {
                return None;
            }
            Some(InjectedFragment {
                content: content.clone(),
                priority: oc.injection_priority.unwrap_or(50),
                visibility: oc
                    .injection_visibility
                    .clone()
                    .unwrap_or(LayerVisibility::Visible),
                source_label: format!("hook:UserPromptSubmit:{idx}"),
            })
        })
        .collect();
    out.sort_by_key(|f| f.priority); // stable sort: ties 保留传入顺序
    out
}

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
                priority: _,
                visibility: _,
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
                    injection_priority: None,
                    injection_visibility: None,
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
                    ShellCommandResult {
                        continue_execution: true,
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
