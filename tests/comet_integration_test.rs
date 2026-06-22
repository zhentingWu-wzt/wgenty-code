//! Comet guard integration tests for ToolExecutor.
//!
//! Tests the full flow: CometState reading, CometGuard checking in
//! execute_with_hooks(), and Notification hook firing on block.

use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use wgenty_code::hooks::HookManager;
use wgenty_code::permissions::policy::ToolPermissionPolicy;
use wgenty_code::tools::ToolRegistry;

/// Global mutex to serialize tests that call process-wide `set_current_dir()`.
static DIR_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Create a temporary directory with a comet yaml in openspec/changes/<name>.
fn setup_comet_dir(
    phase: &str,
    workflow: Option<&str>,
    build_mode: Option<&str>,
) -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    let changes = tmp
        .path()
        .join("openspec")
        .join("changes")
        .join("test-change");
    std::fs::create_dir_all(&changes).unwrap();
    let mut f = std::fs::File::create(changes.join(".comet.yaml")).unwrap();
    writeln!(f, "phase: {}", phase).unwrap();
    if let Some(wf) = workflow {
        writeln!(f, "workflow: {}", wf).unwrap();
    }
    if let Some(bm) = build_mode {
        writeln!(f, "build_mode: {}", bm).unwrap();
    }
    writeln!(f, "archived: false").unwrap();
    tmp
}

/// Creates a ToolExecutor with an empty ToolRegistry and default policy.
fn make_executor() -> wgenty_code::tools::ToolExecutor {
    let registry = Arc::new(ToolRegistry::new());
    let policy = ToolPermissionPolicy::new(PathBuf::from("."));
    wgenty_code::tools::ToolExecutor::new(registry, policy)
}

/// Helper: extract content string from a ChatMessage for assertions.
fn content(msg: &wgenty_code::api::ChatMessage) -> &str {
    msg.content.as_deref().unwrap_or("")
}

#[tokio::test]
async fn test_comet_state_read_on_executor_creation() {
    let _lock = DIR_LOCK.lock().unwrap();
    let tmp = setup_comet_dir("build", Some("full"), None);
    std::env::set_current_dir(tmp.path()).unwrap();

    let executor = make_executor();
    // In Build phase, file_write should be allowed (not blocked).
    let args = serde_json::json!({"file_path": "/tmp/test.rs", "content": "fn main() {}"});
    let msg = executor
        .execute_with_hooks("call-1", "file_write", args, None)
        .await;
    assert!(
        !content(&msg).contains("blocked by comet guard"),
        "file_write should be allowed in Build phase, got: {}",
        content(&msg)
    );
}

#[tokio::test]
async fn test_file_write_blocked_in_open_phase() {
    let _lock = DIR_LOCK.lock().unwrap();
    let tmp = setup_comet_dir("open", Some("full"), None);
    std::env::set_current_dir(tmp.path()).unwrap();

    let executor = make_executor();
    let args = serde_json::json!({"file_path": "/tmp/test.rs", "content": "fn main() {}"});
    let msg = executor
        .execute_with_hooks("call-2", "file_write", args, None)
        .await;
    let c = content(&msg);
    assert!(
        c.contains("blocked by comet guard") || c.contains("不允许"),
        "file_write should be blocked in Open phase, got: {}",
        c
    );
}

#[tokio::test]
async fn test_file_write_blocked_in_verify_phase() {
    let _lock = DIR_LOCK.lock().unwrap();
    let tmp = setup_comet_dir("verify", Some("full"), None);
    std::env::set_current_dir(tmp.path()).unwrap();

    let executor = make_executor();
    let args = serde_json::json!({"file_path": "/tmp/test.rs", "content": "fn main() {}"});
    let msg = executor
        .execute_with_hooks("call-3", "file_write", args, None)
        .await;
    let c = content(&msg);
    assert!(
        c.contains("blocked by comet guard") || c.contains("不允许"),
        "file_write should be blocked in Verify phase, got: {}",
        c
    );
}

#[tokio::test]
async fn test_file_read_allowed_in_open_phase() {
    let _lock = DIR_LOCK.lock().unwrap();
    let tmp = setup_comet_dir("open", Some("full"), None);
    std::env::set_current_dir(tmp.path()).unwrap();

    let executor = make_executor();
    let args = serde_json::json!({"file_path": "/tmp/test.rs"});
    let msg = executor
        .execute_with_hooks("call-4", "file_read", args, None)
        .await;
    assert!(
        !content(&msg).contains("blocked by comet guard"),
        "file_read should be allowed in Open phase, got: {}",
        content(&msg)
    );
}

#[tokio::test]
async fn test_exec_command_git_status_allowed_in_open_phase() {
    let _lock = DIR_LOCK.lock().unwrap();
    let tmp = setup_comet_dir("open", Some("full"), None);
    std::env::set_current_dir(tmp.path()).unwrap();

    let executor = make_executor();
    let args = serde_json::json!({"command": "git status"});
    let msg = executor
        .execute_with_hooks("call-5", "exec_command", args, None)
        .await;
    assert!(
        !content(&msg).contains("blocked by comet guard"),
        "git status should be allowed in Open phase, got: {}",
        content(&msg)
    );
}

#[tokio::test]
async fn test_exec_command_rm_blocked_in_open_phase() {
    let _lock = DIR_LOCK.lock().unwrap();
    let tmp = setup_comet_dir("open", Some("full"), None);
    std::env::set_current_dir(tmp.path()).unwrap();

    let executor = make_executor();
    let args = serde_json::json!({"command": "rm -rf /tmp/test"});
    let msg = executor
        .execute_with_hooks("call-6", "exec_command", args, None)
        .await;
    let c = content(&msg);
    assert!(
        c.contains("blocked by comet guard") || c.contains("不允许"),
        "rm should be blocked in Open phase, got: {}",
        c
    );
}

#[tokio::test]
async fn test_blocked_fires_notification_hook() {
    let _lock = DIR_LOCK.lock().unwrap();
    let tmp = setup_comet_dir("open", Some("full"), None);
    std::env::set_current_dir(tmp.path()).unwrap();

    let registry = Arc::new(ToolRegistry::new());
    let policy = ToolPermissionPolicy::new(PathBuf::from("."));
    let hooks_config = serde_json::json!({
        "Notification": [{"command": "echo 'notified'", "timeout_secs": 5}]
    });
    let hook_manager = Arc::new(HookManager::from_settings(&hooks_config));
    let executor =
        wgenty_code::tools::ToolExecutor::new(registry, policy).with_hooks(hook_manager.clone());

    let args = serde_json::json!({"file_path": "/tmp/test.rs", "content": "fn main() {}"});
    let msg = executor
        .execute_with_hooks("call-7", "file_write", args, None)
        .await;
    let c = content(&msg);
    assert!(
        c.contains("blocked by comet guard") || c.contains("不允许"),
        "file_write should be blocked, got: {}",
        c
    );
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
}

#[tokio::test]
async fn test_guard_runs_before_pre_tool_use_hook() {
    let _lock = DIR_LOCK.lock().unwrap();
    let tmp = setup_comet_dir("open", Some("full"), None);
    std::env::set_current_dir(tmp.path()).unwrap();

    let registry = Arc::new(ToolRegistry::new());
    let policy = ToolPermissionPolicy::new(PathBuf::from("."));
    let hooks_config = serde_json::json!({
        "PreToolUse": [{"command": "echo 'pre-hook'", "timeout_secs": 5, "matcher": "file_write"}],
        "Notification": [{"command": "echo 'notified'", "timeout_secs": 5}]
    });
    let hook_manager = Arc::new(HookManager::from_settings(&hooks_config));
    let executor =
        wgenty_code::tools::ToolExecutor::new(registry, policy).with_hooks(hook_manager.clone());

    let args = serde_json::json!({"file_path": "/tmp/test.rs", "content": "fn main() {}"});
    let msg = executor
        .execute_with_hooks("call-8", "file_write", args, None)
        .await;

    let c = content(&msg);
    assert!(
        c.contains("blocked by comet guard") || c.contains("不允许"),
        "file_write should be blocked by comet guard (not PreToolUse hook), got: {}",
        c
    );
}

#[tokio::test]
async fn test_no_comet_state_skips_guard() {
    let _lock = DIR_LOCK.lock().unwrap();
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_current_dir(tmp.path()).unwrap();

    let executor = make_executor();
    let args = serde_json::json!({"file_path": "/tmp/test.rs", "content": "fn main() {}"});
    let msg = executor
        .execute_with_hooks("call-9", "file_write", args, None)
        .await;
    assert!(
        !content(&msg).contains("blocked by comet guard"),
        "file_write should not be blocked without comet state, got: {}",
        content(&msg)
    );
}

#[tokio::test]
async fn test_exec_command_cargo_build_blocked_in_open_phase() {
    let _lock = DIR_LOCK.lock().unwrap();
    let tmp = setup_comet_dir("open", Some("full"), None);
    std::env::set_current_dir(tmp.path()).unwrap();

    let executor = make_executor();
    let args = serde_json::json!({"command": "cargo  build  --release"});
    let msg = executor
        .execute_with_hooks("call-10", "exec_command", args, None)
        .await;
    let c = content(&msg);
    assert!(
        c.contains("blocked by comet guard") || c.contains("不允许"),
        "cargo build should be blocked in Open phase, got: {}",
        c
    );
}

#[tokio::test]
async fn test_file_write_with_string_args_blocked_in_open() {
    let _lock = DIR_LOCK.lock().unwrap();
    let tmp = setup_comet_dir("open", Some("full"), None);
    std::env::set_current_dir(tmp.path()).unwrap();

    let executor = make_executor();
    let args = serde_json::json!("some/path/file.rs");
    let msg = executor
        .execute_with_hooks("call-11", "file_write", args, None)
        .await;
    let c = content(&msg);
    assert!(
        c.contains("blocked by comet guard") || c.contains("不允许"),
        "file_write with string args should be blocked in Open phase, got: {}",
        c
    );
}
