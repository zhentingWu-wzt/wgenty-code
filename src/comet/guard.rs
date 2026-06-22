//! Comet guard — phase restriction matrix for tool calls.
//!
//! Enforces that mutating operations (file_write, file_edit, apply_patch, and
//! mutating Bash commands) are only allowed during the Build phase.
//! Read-only operations are permitted in all phases.

use crate::comet::state::{CometPhase, CometState};
use std::path::Path;

/// Decision from a phase-guard check.
#[derive(Debug, Clone)]
pub struct CometGuardDecision {
    pub blocked: bool,
    pub error_message: Option<String>,
    pub phase: CometPhase,
}

/// Phase-aware guard for tool call restrictions.
pub struct CometGuard;

impl CometGuard {
    /// Check whether a tool call is allowed in the given phase.
    ///
    /// Build phase allows all operations. All other phases only allow
    /// read-only tools and read-only Bash commands.
    pub fn check(phase: &CometPhase, tool_name: &str, args: &[String]) -> CometGuardDecision {
        // Build phase allows everything.
        if *phase == CometPhase::Build {
            return CometGuardDecision {
                blocked: false,
                error_message: None,
                phase: phase.clone(),
            };
        }

        // Inherently read-only tools are always allowed.
        if is_read_only(tool_name) {
            return CometGuardDecision {
                blocked: false,
                error_message: None,
                phase: phase.clone(),
            };
        }

        // Shell commands are special: distinguish read-only vs mutating by the command.
        if (tool_name == "exec_command" || tool_name == "execute_command")
            && is_read_only_bash_command(args)
        {
            return CometGuardDecision {
                blocked: false,
                error_message: None,
                phase: phase.clone(),
            };
        }

        // Everything else is blocked in non-Build phases.
        CometGuardDecision {
            blocked: true,
            error_message: Some(format!(
                "当前处于 {:?} 阶段，不允许执行此操作（{}）。只有 Build 阶段允许修改源代码。",
                phase, tool_name
            )),
            phase: phase.clone(),
        }
    }

    /// Check whether the active change is in coordinator (subagent-driven) mode.
    pub fn is_coordinator_mode(working_dir: &Path) -> bool {
        if let Some(state) = CometState::read(working_dir) {
            state.build_mode.as_deref() == Some("subagent-driven-development")
        } else {
            false
        }
    }

    /// Return a static reminder text for coordinator mode.
    pub fn coordinator_reminder() -> &'static str {
        "你是协调者（coordinator），不是执行者。请使用 subagent-driven-development 流程，禁止在主会话中直接执行 task。"
    }
}

/// Returns true if the named tool is inherently read-only.
///
/// Tool names match the actual `Tool::name()` values from `src/tools/`.
/// This list is kept in sync with tools that return `true` from
/// `Tool::is_read_only()`, plus meta-tools (`ask_user_question`,
/// `update_plan`) that do not mutate source code.
pub fn is_read_only(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "file_read"
            | "web_search"
            | "web_fetch"
            | "skill"
            | "think"
            | "compact"
            | "lsp"
            | "list_files"
            | "glob"
            | "view"
            | "grep"
            | "search"
            | "load_skill"
            | "codegraph_node"
            | "codegraph_explore"
            | "call_path"
            | "symbol_batch"
            | "module_summary"
            | "ask_user_question"
            | "update_plan"
    )
}

/// Returns true if the named tool is an inherently mutating tool
/// (`file_write`, `file_edit`, `apply_patch`). Shell tools
/// (`exec_command` / `execute_command`) are NOT inherently mutating —
/// it depends on the command being run.
pub fn is_mutating_command(tool_name: &str) -> bool {
    matches!(tool_name, "file_write" | "file_edit" | "apply_patch")
}

/// Returns true if the Bash command arguments describe a read-only operation.
///
/// Recognises common read-only commands like `git status`, `ls`, `cat`, etc.
fn is_read_only_bash_command(args: &[String]) -> bool {
    if args.is_empty() {
        return false;
    }

    // git subcommands that are read-only.
    if args[0] == "git" && args.len() >= 2 {
        // git stash bare (no subcommand) modifies the working tree.
        // Only `git stash list` and `git stash show` are read-only.
        if args[1] == "stash" {
            return args.len() >= 3 && matches!(args[2].as_str(), "list" | "show");
        }
        return matches!(
            args[1].as_str(),
            "status" | "log" | "diff" | "branch" | "show" | "remote" | "tag"
        );
    }

    // Common read-only unix commands.
    matches!(
        args[0].as_str(),
        "ls" | "cat" | "head" | "tail" | "find" | "grep" | "rg" | "wc" | "file" | "which"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn read_args() -> Vec<String> {
        vec!["/some/file.rs".to_string()]
    }

    fn write_args() -> Vec<String> {
        vec!["/some/file.rs".to_string(), "content".to_string()]
    }

    fn bash_args(cmd: &[&str]) -> Vec<String> {
        cmd.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn test_file_read_allowed_in_open() {
        let d = CometGuard::check(&CometPhase::Open, "file_read", &read_args());
        assert!(!d.blocked, "file_read in Open should be allowed");
    }

    #[test]
    fn test_file_write_blocked_in_open() {
        let d = CometGuard::check(&CometPhase::Open, "file_write", &write_args());
        assert!(d.blocked, "file_write in Open should be blocked");
        assert!(d.error_message.is_some());
    }

    #[test]
    fn test_file_write_allowed_in_build() {
        let d = CometGuard::check(&CometPhase::Build, "file_write", &write_args());
        assert!(!d.blocked, "file_write in Build should be allowed");
    }

    #[test]
    fn test_file_write_blocked_in_verify() {
        let d = CometGuard::check(&CometPhase::Verify, "file_write", &write_args());
        assert!(d.blocked, "file_write in Verify should be blocked");
    }

    #[test]
    fn test_git_status_allowed_in_all_phases() {
        let args = bash_args(&["git", "status"]);
        for phase in &[
            CometPhase::Open,
            CometPhase::Design,
            CometPhase::Build,
            CometPhase::Verify,
            CometPhase::Archive,
        ] {
            let d = CometGuard::check(phase, "exec_command", &args);
            assert!(
                !d.blocked,
                "git status should be allowed in {:?}",
                phase
            );
        }
    }

    #[test]
    fn test_mutating_bash_blocked_in_open() {
        let args = bash_args(&["rm", "-rf", "/tmp/test"]);
        let d = CometGuard::check(&CometPhase::Open, "exec_command", &args);
        assert!(d.blocked, "rm -rf in Open should be blocked");
    }

    #[test]
    fn test_mutating_bash_allowed_in_build() {
        let args = bash_args(&["cargo", "build"]);
        let d = CometGuard::check(&CometPhase::Build, "exec_command", &args);
        assert!(!d.blocked, "cargo build in Build should be allowed");
    }

    #[test]
    fn test_edit_blocked_in_open() {
        let d = CometGuard::check(
            &CometPhase::Open,
            "file_edit",
            &["/file.rs".to_string(), "old".to_string(), "new".to_string()],
        );
        assert!(d.blocked, "file_edit in Open should be blocked");
    }

    #[test]
    fn test_edit_allowed_in_build() {
        let d = CometGuard::check(
            &CometPhase::Build,
            "file_edit",
            &["/file.rs".to_string(), "old".to_string(), "new".to_string()],
        );
        assert!(!d.blocked, "file_edit in Build should be allowed");
    }

    #[test]
    fn test_coordinator_reminder_non_empty() {
        let r = CometGuard::coordinator_reminder();
        assert!(!r.is_empty());
        assert!(r.contains("coordinator") || r.contains("协调者"));
    }

    #[test]
    fn test_is_coordinator_mode_no_active_change() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(!CometGuard::is_coordinator_mode(tmp.path()));
    }

    #[test]
    fn test_is_coordinator_mode_with_subagent() {
        let tmp = tempfile::tempdir().unwrap();
        let changes = tmp.path().join("openspec").join("changes").join("my-change");
        std::fs::create_dir_all(&changes).unwrap();
        let mut f = std::fs::File::create(changes.join(".comet.yaml")).unwrap();
        writeln!(f, "phase: build").unwrap();
        writeln!(f, "build_mode: subagent-driven-development").unwrap();
        writeln!(f, "archived: false").unwrap();
        assert!(CometGuard::is_coordinator_mode(tmp.path()));
    }

    #[test]
    fn test_is_coordinator_mode_not_coordinator() {
        let tmp = tempfile::tempdir().unwrap();
        let changes = tmp.path().join("openspec").join("changes").join("my-change");
        std::fs::create_dir_all(&changes).unwrap();
        let mut f = std::fs::File::create(changes.join(".comet.yaml")).unwrap();
        writeln!(f, "phase: build").unwrap();
        writeln!(f, "build_mode: direct").unwrap();
        writeln!(f, "archived: false").unwrap();
        assert!(!CometGuard::is_coordinator_mode(tmp.path()));
    }

    #[test]
    fn test_is_read_only_tools() {
        assert!(is_read_only("file_read"));
        assert!(is_read_only("web_search"));
        assert!(is_read_only("web_fetch"));
        assert!(is_read_only("skill"));
        assert!(is_read_only("think"));
        assert!(is_read_only("compact"));
        assert!(is_read_only("lsp"));
        assert!(is_read_only("list_files"));
        assert!(is_read_only("glob"));
        assert!(is_read_only("view"));
        assert!(is_read_only("grep"));
        assert!(is_read_only("search"));
        assert!(is_read_only("load_skill"));
        assert!(is_read_only("ask_user_question"));
        assert!(is_read_only("update_plan"));
        assert!(!is_read_only("file_write"));
        assert!(!is_read_only("file_edit"));
        assert!(!is_read_only("apply_patch"));
        assert!(!is_read_only("exec_command"));
        assert!(!is_read_only("execute_command"));
    }

    #[test]
    fn test_is_mutating_command() {
        assert!(is_mutating_command("file_write"));
        assert!(is_mutating_command("file_edit"));
        assert!(is_mutating_command("apply_patch"));
        assert!(!is_mutating_command("file_read"));
        assert!(!is_mutating_command("exec_command"));
        assert!(!is_mutating_command("web_search"));
    }

    #[test]
    fn test_read_only_bash_commands_allowed_in_open() {
        let cmds: &[&[&str]] = &[
            &["git", "status"],
            &["git", "log"],
            &["git", "diff"],
            &["git", "branch"],
            &["ls"],
            &["cat", "file.txt"],
            &["find", ".", "-name", "*.rs"],
        ];
        for cmd in cmds {
            let args = bash_args(cmd);
            let d = CometGuard::check(&CometPhase::Open, "exec_command", &args);
            assert!(!d.blocked, "{:?} should be allowed in Open", cmd);
        }
    }

    #[test]
    fn test_git_stash_list_is_read_only() {
        // git stash list: read-only, allowed in Open
        let args = bash_args(&["git", "stash", "list"]);
        let d = CometGuard::check(&CometPhase::Open, "exec_command", &args);
        assert!(!d.blocked, "git stash list should be allowed in Open");
    }

    #[test]
    fn test_git_stash_show_is_read_only() {
        // git stash show: read-only, allowed in Open
        let args = bash_args(&["git", "stash", "show"]);
        let d = CometGuard::check(&CometPhase::Open, "exec_command", &args);
        assert!(!d.blocked, "git stash show should be allowed in Open");
    }

    #[test]
    fn test_git_stash_bare_is_mutating() {
        // git stash (no subcommand): modifies working tree, blocked in Open
        let args = bash_args(&["git", "stash"]);
        let d = CometGuard::check(&CometPhase::Open, "exec_command", &args);
        assert!(d.blocked, "git stash bare should be blocked in Open");
    }

    #[test]
    fn test_git_stash_pop_is_mutating() {
        // git stash pop: modifies working tree, blocked in Open
        let args = bash_args(&["git", "stash", "pop"]);
        let d = CometGuard::check(&CometPhase::Open, "exec_command", &args);
        assert!(d.blocked, "git stash pop should be blocked in Open");
    }
}
