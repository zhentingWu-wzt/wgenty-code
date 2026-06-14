//! CC hooks format adapter — converts nested CC hooks arrays to flat HookDefinition list.
//!
//! CC format: { "PostToolUse": [[{ "type": "command", "command": "...", "matcher": "..." }]] }
//! Internal format: HashMap<HookEvent, Vec<HookDefinition>>

use super::{HookDefinition, HookEvent};
use serde::Deserialize;
use std::collections::HashMap;

/// Raw CC hook item from settings.json
#[derive(Debug, Deserialize)]
pub struct CcHookItem {
    pub r#type: String,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub matcher: Option<String>,
    #[serde(default)]
    pub env: Option<HashMap<String, String>>,
    #[serde(default)]
    pub timeout: Option<u64>,
}

/// Adapt CC-format hooks (nested arrays) to flat HashMap<HookEvent, Vec<HookDefinition>>.
///
/// CC format outer structure:
/// ```json
/// {
///   "PostToolUse": [[{ "type": "command", ... }]],
///   "Stop": [[{ "type": "prompt", ... }]]
/// }
/// ```
pub fn adapt_cc_hooks(
    hooks_config: &serde_json::Value,
) -> HashMap<HookEvent, Vec<HookDefinition>> {
    let mut hooks: HashMap<HookEvent, Vec<HookDefinition>> = HashMap::new();

    let obj = match hooks_config.as_object() {
        Some(o) => o,
        None => return hooks,
    };

    for (event_name, event_hooks) in obj {
        let event = match parse_hook_event(event_name) {
            Some(e) => e,
            None => continue,
        };

        // CC: event_hooks is Vec<Vec<CcHookItem>>
        // Outer Vec = independent hook groups, Inner Vec = sequential sub-hooks
        let mut definitions = Vec::new();

        if let Some(groups) = event_hooks.as_array() {
            for group in groups {
                if let Some(items) = group.as_array() {
                    for item in items {
                        if let Ok(hook_item) = serde_json::from_value::<CcHookItem>(item.clone()) {
                            let command = match hook_item.r#type.as_str() {
                                "command" => hook_item.command.unwrap_or_default(),
                                "prompt" => hook_item.prompt.unwrap_or_default(),
                                _ => continue,
                            };

                            definitions.push(HookDefinition {
                                command,
                                timeout_secs: hook_item.timeout.unwrap_or(30),
                                matcher: hook_item.matcher,
                                hook_type: Some(hook_item.r#type),
                            });
                        }
                    }
                }
            }
        }

        if !definitions.is_empty() {
            hooks.insert(event, definitions);
        }
    }

    hooks
}

/// Parse a CC hook event name into a HookEvent variant.
fn parse_hook_event(name: &str) -> Option<HookEvent> {
    match name {
        "PreToolUse" => Some(HookEvent::PreToolUse),
        "PostToolUse" => Some(HookEvent::PostToolUse),
        "SessionStart" => Some(HookEvent::SessionStart),
        "SessionEnd" => Some(HookEvent::SessionEnd),
        "Notification" => Some(HookEvent::Notification),
        "Stop" => Some(HookEvent::Stop),
        "UserPromptSubmit" => Some(HookEvent::UserPromptSubmit),
        "PermissionRequest" => Some(HookEvent::PermissionRequest),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_adapt_cc_hooks_basic() {
        let json = serde_json::json!({
            "Stop": [[
                {"type": "command", "command": "echo 'session ended'"}
            ]]
        });

        let hooks = adapt_cc_hooks(&json);
        assert!(hooks.contains_key(&HookEvent::Stop));
        let defs = &hooks[&HookEvent::Stop];
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].command, "echo 'session ended'");
        assert_eq!(defs[0].hook_type.as_deref(), Some("command"));
    }

    #[test]
    fn test_adapt_cc_hooks_with_matcher() {
        let json = serde_json::json!({
            "PostToolUse": [[
                {"type": "command", "command": "python3 analyze.py", "matcher": "TaskCreate|TaskUpdate"}
            ]]
        });

        let hooks = adapt_cc_hooks(&json);
        let defs = &hooks[&HookEvent::PostToolUse];
        assert_eq!(defs[0].matcher.as_deref(), Some("TaskCreate|TaskUpdate"));
    }

    #[test]
    fn test_adapt_cc_hooks_unknown_event() {
        let json = serde_json::json!({
            "UnknownEvent": [[{"type": "command", "command": "echo test"}]]
        });
        let hooks = adapt_cc_hooks(&json);
        assert!(hooks.is_empty());
    }
}
