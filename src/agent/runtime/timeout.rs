//! Tool-call timeout policy shared by every agent loop path.

use std::cmp;
use std::time::Duration;

/// Resolve the timeout for a tool call based on its name and arguments.
///
/// | tool                         | timeout                                              |
/// |------------------------------|------------------------------------------------------|
/// | `task` / `delegate`          | `subagent_timeout_secs + 120s` buffer                |
/// | `execute_command` / `exec`   | `max(args.timeout + 30, 120)` where timeout defaults to 60 |
/// | all other tools              | 120s                                                 |
///
/// `subagent_timeout_secs` must always be > the subagent's own loop timeout
/// so the subagent reports its own timeout before the parent cuts the connection.
pub fn resolve_tool_timeout(
    tool_name: &str,
    args: &serde_json::Value,
    subagent_timeout_secs: u64,
) -> Duration {
    match tool_name {
        "task" | "delegate" => Duration::from_secs(subagent_timeout_secs.saturating_add(120)),
        "execute_command" | "exec_command" => {
            let user_timeout = args.get("timeout").and_then(|v| v.as_u64()).unwrap_or(60);
            Duration::from_secs(cmp::max(user_timeout + 30, 120))
        }
        _ => Duration::from_secs(120),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::Duration;

    #[test]
    fn test_timeout_task() {
        let args = json!({});
        // subagent_timeout_secs = default 1800 → 1800 + 120 = 1920
        assert_eq!(
            resolve_tool_timeout("task", &args, 1800),
            Duration::from_secs(1920)
        );
    }

    #[test]
    fn test_timeout_delegate() {
        let args = json!({});
        assert_eq!(
            resolve_tool_timeout("delegate", &args, 1800),
            Duration::from_secs(1920)
        );
    }

    #[test]
    fn test_timeout_task_with_custom_subagent_timeout() {
        let args = json!({});
        // subagent_timeout_secs = 600 → 600 + 120 = 720
        assert_eq!(
            resolve_tool_timeout("task", &args, 600),
            Duration::from_secs(720)
        );
    }

    #[test]
    fn test_timeout_execute_command_with_default_timeout() {
        // No timeout in args → defaults to 60 → max(60+30, 120) = 120
        // subagent_timeout_secs is irrelevant for exec_command, use 1800
        let args = json!({"command": "echo hello"});
        assert_eq!(
            resolve_tool_timeout("execute_command", &args, 1800),
            Duration::from_secs(120)
        );
    }

    #[test]
    fn test_timeout_execute_command_with_custom_timeout_under_min() {
        // timeout=30 → max(30+30, 120) = 120
        let args = json!({"command": "echo hello", "timeout": 30});
        assert_eq!(
            resolve_tool_timeout("execute_command", &args, 1800),
            Duration::from_secs(120)
        );
    }

    #[test]
    fn test_timeout_execute_command_with_large_timeout() {
        // timeout=200 → max(200+30, 120) = 230
        let args = json!({"command": "echo hello", "timeout": 200});
        assert_eq!(
            resolve_tool_timeout("execute_command", &args, 1800),
            Duration::from_secs(230)
        );
    }

    #[test]
    fn test_timeout_exec_command_alias() {
        let args = json!({"command": "echo hello", "timeout": 100});
        assert_eq!(
            resolve_tool_timeout("exec_command", &args, 1800),
            Duration::from_secs(130)
        );
    }

    #[test]
    fn test_timeout_other_tool() {
        let args = json!({});
        assert_eq!(
            resolve_tool_timeout("Bash", &args, 1800),
            Duration::from_secs(120)
        );
        assert_eq!(
            resolve_tool_timeout("Read", &args, 1800),
            Duration::from_secs(120)
        );
        assert_eq!(
            resolve_tool_timeout("WebSearch", &args, 1800),
            Duration::from_secs(120)
        );
    }
}
