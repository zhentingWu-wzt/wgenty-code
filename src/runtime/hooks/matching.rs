use super::types::HookEvent;

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
pub(super) fn shell_escape(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}
