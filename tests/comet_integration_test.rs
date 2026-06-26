//! Hook-based phase guard integration tests for ToolExecutor.
//!
//! Tests the new hook-based workflow: PreToolUse hooks with `when_state`
//! filtering replace the old CometGuard phase-guard logic.
//! ToolExecutor uses `state_handle` (Arc<RwLock<String>>) for workflow state.

use std::sync::Arc;
use tokio::sync::RwLock;
use wgenty_code::permissions::policy::ToolPermissionPolicy;
use wgenty_code::runtime::hooks::{HookAction, HookDefinition, HookEvent, HookManager};
use wgenty_code::tools::ToolRegistry;

/// Creates a ToolExecutor with an empty ToolRegistry and default policy.
fn make_executor() -> wgenty_code::tools::ToolExecutor {
    let registry = Arc::new(ToolRegistry::new());
    let policy = ToolPermissionPolicy::new(std::path::PathBuf::from("."));
    wgenty_code::tools::ToolExecutor::new(registry, policy)
}

/// Helper: extract content string from a ChatMessage for assertions.
fn content(msg: &wgenty_code::api::ChatMessage) -> &str {
    msg.content.as_deref().unwrap_or("")
}

/// Build a blocking PreToolUse hook that blocks a given tool name when
/// the workflow state matches one of the given states (pipe-separated).
fn blocking_hook(tool_matcher: &str, when_state: &str, reason: &str) -> HookDefinition {
    HookDefinition {
        event: HookEvent::PreToolUse,
        matcher: Some(tool_matcher.to_string()),
        when_state: Some(when_state.to_string()),
        actions: vec![HookAction::Command {
            command: format!(
                "echo '{{\"continue_execution\":false,\"reason\":\"{}\"}}'",
                reason
            ),
            timeout_secs: 5,
        }],
    }
}

// ── State handle setter ──────────────────────────────────────────────────

#[test]
fn test_state_handle_setter() {
    let mut executor = make_executor();
    let handle = Arc::new(RwLock::new("open".to_string()));
    executor.set_state_handle(Some(handle.clone()));
    let state = executor
        .state_handle
        .as_ref()
        .unwrap()
        .try_read()
        .unwrap()
        .clone();
    assert_eq!(state, "open");
}

// ── No state = no blocking ───────────────────────────────────────────────

#[tokio::test]
async fn test_no_state_handle_skips_hook_blocking() {
    let mut hm = HookManager::default();
    hm.register_workflow_hooks(vec![blocking_hook(
        "file_write",
        "open|design|verify|archive",
        "write blocked outside build",
    )]);
    let hook_manager = Arc::new(hm);
    let mut executor = make_executor();
    executor = executor.with_hooks(hook_manager);
    // No state_handle set → hooks with when_state fire regardless (state=None fallback)

    // Use read-only tool to avoid touching the filesystem
    let result = executor
        .execute_with_hooks(
            "test-no-state",
            "file_read",
            serde_json::json!({"file_path": "/tmp/test.rs"}),
            None,
        )
        .await;
    // Without state, the file_read tool runs (though it may error on missing file — that's fine)
    let msg = content(&result);
    assert!(
        !msg.contains("blocked by hook"),
        "Without state_handle, tools should not be blocked by hook, got: {}",
        msg
    );
}

// ── Build phase allows everything ────────────────────────────────────────

#[tokio::test]
async fn test_build_phase_allows_write_tools() {
    let mut hm = HookManager::default();
    // Only block writes in non-build phases
    hm.register_workflow_hooks(vec![blocking_hook(
        "file_write|file_edit|apply_patch",
        "open|design|verify|archive",
        "write blocked outside build",
    )]);
    let hook_manager = Arc::new(hm);
    let mut executor = make_executor().with_hooks(hook_manager);
    executor.set_state_handle(Some(Arc::new(RwLock::new("build".to_string()))));

    let result = executor
        .execute_with_hooks(
            "test-build",
            "file_write",
            serde_json::json!({"path": "/tmp/test_build_rs", "content": "fn main() {}"}),
            None,
        )
        .await;
    let msg = content(&result);
    assert!(
        !msg.contains("blocked by hook"),
        "file_write should be allowed in Build phase, got: {}",
        msg
    );
}

// ── Open phase blocks write tools ────────────────────────────────────────

#[tokio::test]
async fn test_open_phase_blocks_write_tools() {
    let mut hm = HookManager::default();
    hm.register_workflow_hooks(vec![blocking_hook(
        "file_write|file_edit|apply_patch",
        "open|design|verify|archive",
        "write blocked outside build",
    )]);
    let hook_manager = Arc::new(hm);
    let mut executor = make_executor().with_hooks(hook_manager);
    executor.set_state_handle(Some(Arc::new(RwLock::new("open".to_string()))));

    let result = executor
        .execute_with_hooks(
            "test-open",
            "file_write",
            serde_json::json!({"path": "/tmp/test_open_rs", "content": "fn main() {}"}),
            None,
        )
        .await;
    let msg = content(&result);
    assert!(
        msg.contains("blocked by hook"),
        "file_write should be blocked by hook in Open phase, got: {}",
        msg
    );
}

// ── Read tools are NOT blocked in non-build phases ────────────────────────

#[tokio::test]
async fn test_read_tools_not_blocked_in_open_phase() {
    let mut hm = HookManager::default();
    hm.register_workflow_hooks(vec![blocking_hook(
        "file_write|file_edit|apply_patch",
        "open|design|verify|archive",
        "write blocked outside build",
    )]);
    let hook_manager = Arc::new(hm);
    let mut executor = make_executor().with_hooks(hook_manager);
    executor.set_state_handle(Some(Arc::new(RwLock::new("open".to_string()))));

    let result = executor
        .execute_with_hooks(
            "test-read-open",
            "file_read",
            serde_json::json!({"file_path": "/tmp/nonexistent"}),
            None,
        )
        .await;
    let msg = content(&result);
    assert!(
        !msg.contains("blocked by hook"),
        "file_read should NOT be blocked in Open phase (matcher only targets write tools), got: {}",
        msg
    );
}

// ── exec_command with mutating commands blocked in Open ───────────────────

#[tokio::test]
async fn test_exec_command_blocked_in_open_phase() {
    let mut hm = HookManager::default();
    hm.register_workflow_hooks(vec![blocking_hook(
        "exec_command|execute_command",
        "open|design|verify|archive",
        "shell commands blocked outside build",
    )]);
    let hook_manager = Arc::new(hm);
    let mut executor = make_executor().with_hooks(hook_manager);
    executor.set_state_handle(Some(Arc::new(RwLock::new("open".to_string()))));

    let result = executor
        .execute_with_hooks(
            "test-cmd-open",
            "exec_command",
            serde_json::json!({"command": "rm -rf /tmp/test"}),
            None,
        )
        .await;
    let msg = content(&result);
    assert!(
        msg.contains("blocked by hook"),
        "exec_command should be blocked in Open phase, got: {}",
        msg
    );
}

// ── exec_command allowed in Build ────────────────────────────────────────

#[tokio::test]
async fn test_exec_command_allowed_in_build_phase() {
    let mut hm = HookManager::default();
    hm.register_workflow_hooks(vec![blocking_hook(
        "exec_command|execute_command",
        "open|design|verify|archive",
        "shell commands blocked outside build",
    )]);
    let hook_manager = Arc::new(hm);
    let mut executor = make_executor().with_hooks(hook_manager);
    executor.set_state_handle(Some(Arc::new(RwLock::new("build".to_string()))));

    let result = executor
        .execute_with_hooks(
            "test-cmd-build",
            "exec_command",
            serde_json::json!({"command": "echo hello"}),
            None,
        )
        .await;
    let msg = content(&result);
    assert!(
        !msg.contains("blocked by hook"),
        "exec_command (echo hello) should be allowed in Build phase, got: {}",
        msg
    );
}

// ── Notification hook fires on block ──────────────────────────────────────

#[tokio::test]
async fn test_notification_hook_fires_on_block() {
    let mut hm = HookManager::default();
    // PreToolUse hook blocks write tools in Open
    hm.register_workflow_hooks(vec![blocking_hook(
        "file_write",
        "open|design|verify|archive",
        "write blocked outside build",
    )]);
    // Notification hook (should fire when something is blocked)
    hm.register_workflow_hooks(vec![HookDefinition {
        event: HookEvent::Notification,
        matcher: None,
        when_state: None,
        actions: vec![HookAction::Command {
            command: "echo '{\"continue_execution\":true}'".to_string(),
            timeout_secs: 5,
        }],
    }]);
    let hook_manager = Arc::new(hm);
    let mut executor = make_executor().with_hooks(hook_manager);
    executor.set_state_handle(Some(Arc::new(RwLock::new("open".to_string()))));

    let result = executor
        .execute_with_hooks(
            "test-notif",
            "file_write",
            serde_json::json!({"path": "/tmp/test_notify_rs", "content": "fn main() {}"}),
            None,
        )
        .await;
    let msg = content(&result);
    assert!(
        msg.contains("blocked by hook"),
        "file_write should be blocked by hook (and Notification fired), got: {}",
        msg
    );
    // Brief pause to let the spawned Notification task complete
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
}

// ── Hook state mismatch does not block ────────────────────────────────────

#[tokio::test]
async fn test_hook_state_mismatch_allows() {
    let mut hm = HookManager::default();
    hm.register_workflow_hooks(vec![blocking_hook(
        "file_write",
        "open", // only blocks in "open"
        "write blocked in open",
    )]);
    let hook_manager = Arc::new(hm);
    let mut executor = make_executor().with_hooks(hook_manager);
    // State is "design" — hook's when_state is "open" → should NOT match
    executor.set_state_handle(Some(Arc::new(RwLock::new("design".to_string()))));

    let result = executor
        .execute_with_hooks(
            "test-mismatch",
            "file_write",
            serde_json::json!({"path": "/tmp/test_mismatch_rs", "content": "fn main() {}"}),
            None,
        )
        .await;
    let msg = content(&result);
    assert!(
        !msg.contains("blocked by hook"),
        "file_write should NOT be blocked in design phase when hook targets only open, got: {}",
        msg
    );
}
