//! Tests for Tools Module

use wgenty_code::tools::ToolRegistry;

#[tokio::test]
async fn test_tool_registry_creation() {
    let registry = ToolRegistry::new();
    let tools = registry.list();

    // Should have 9 tools now (6 original + 3 new)
    assert!(tools.len() >= 6);
}

#[tokio::test]
async fn test_file_read_tool() {
    let registry = ToolRegistry::new();
    let tool = registry
        .get("file_read")
        .expect("file_read tool should exist");

    assert_eq!(tool.name(), "file_read");
    assert!(!tool.description().is_empty());
}

#[tokio::test]
async fn test_git_operations_tool() {
    let registry = ToolRegistry::new();
    let tool = registry
        .get("git_operations")
        .expect("git_operations tool should exist");

    assert_eq!(tool.name(), "git_operations");
    assert!(tool.description().contains("Git"));
}

#[tokio::test]
async fn test_task_management_tool() {
    let registry = ToolRegistry::new();
    let tool = registry
        .get("task_management")
        .expect("task_management tool should exist");

    assert_eq!(tool.name(), "task_management");
    assert!(tool.description().contains("task"));
}

#[tokio::test]
async fn test_note_edit_tool() {
    let registry = ToolRegistry::new();
    let tool = registry
        .get("note_edit")
        .expect("note_edit tool should exist");

    assert_eq!(tool.name(), "note_edit");
    assert!(tool.description().contains("note"));
}

#[tokio::test]
async fn test_task_create_and_list() {
    use serde_json::json;

    let registry = ToolRegistry::new();

    // Create a task
    let create_result = registry
        .execute(
            "task_management",
            json!({
                "operation": "create",
                "subject": "Test Task",
                "description": "This is a test task"
            }),
        )
        .await;

    assert!(create_result.is_ok());

    // List tasks
    let list_result = registry
        .execute(
            "task_management",
            json!({
                "operation": "list"
            }),
        )
        .await;

    assert!(list_result.is_ok());
}

#[tokio::test]
async fn test_note_create_and_search() {
    use serde_json::json;

    let registry = ToolRegistry::new();

    // Create a note
    let create_result = registry
        .execute(
            "note_edit",
            json!({
                "operation": "create",
                "title": "Test Note",
                "content": "This is a test note content",
                "tags": ["test", "example"]
            }),
        )
        .await;

    assert!(create_result.is_ok());

    // Search notes
    let search_result = registry
        .execute(
            "note_edit",
            json!({
                "operation": "search",
                "search_query": "test"
            }),
        )
        .await;

    assert!(search_result.is_ok());
}

#[tokio::test]
async fn test_exec_command_session_lifecycle() {
    use serde_json::json;

    let registry = ToolRegistry::new();

    let start_result = registry
        .execute(
            "exec_command",
            json!({
                "command": "printf 'hello'",
                "yield_time_ms": 50,
                "max_output_chars": 200
            }),
        )
        .await
        .expect("exec_command should succeed");

    assert_eq!(start_result.output_type, "text");
    assert!(start_result.content.contains("hello"));

    let session_id = start_result.metadata["session_id"]
        .as_u64()
        .expect("session_id should be present");

    let followup_result = registry
        .execute(
            "write_stdin",
            json!({
                "session_id": session_id,
                "chars": "",
                "yield_time_ms": 10,
                "max_output_chars": 200
            }),
        )
        .await
        .expect("write_stdin should succeed");

    assert_eq!(followup_result.metadata["session_id"], json!(session_id));
}

#[tokio::test]
async fn test_exec_command_interactive_io() {
    use serde_json::json;

    let registry = ToolRegistry::new();

    let start_result = registry
        .execute(
            "exec_command",
            json!({
                "command": "read line; printf '%s' \"$line\"",
                "yield_time_ms": 10,
                "max_output_chars": 200
            }),
        )
        .await
        .expect("exec_command should succeed");

    let session_id = start_result.metadata["session_id"]
        .as_u64()
        .expect("session_id should be present");

    let io_result = registry
        .execute(
            "write_stdin",
            json!({
                "session_id": session_id,
                "chars": "world\n",
                "yield_time_ms": 50,
                "max_output_chars": 200
            }),
        )
        .await
        .expect("write_stdin should succeed");

    assert!(io_result.content.contains("world"));
}

#[tokio::test]
async fn test_kill_session_tool() {
    use serde_json::json;

    let registry = ToolRegistry::new();

    let start_result = registry
        .execute(
            "exec_command",
            json!({
                "command": "sleep 5",
                "yield_time_ms": 10,
                "max_output_chars": 100
            }),
        )
        .await
        .expect("exec_command should succeed");

    let session_id = start_result.metadata["session_id"]
        .as_u64()
        .expect("session_id should be present");

    let kill_result = registry
        .execute(
            "kill_session",
            json!({
                "session_id": session_id
            }),
        )
        .await
        .expect("kill_session should succeed");

    assert!(kill_result.content.contains("Killed session"));
}

#[tokio::test]
async fn test_apply_patch_add_update_delete() {
    use serde_json::json;
    use tempfile::tempdir;

    let dir = tempdir().expect("tempdir should be created");
    let file_path = dir.path().join("sample.txt");
    std::fs::write(&file_path, "alpha\nbeta\n").expect("seed file should be written");

    let registry = ToolRegistry::new();

    let add_result = registry
        .execute(
            "apply_patch",
            json!({
                "patch": "*** Begin Patch\n*** Add File: added.txt\n+hello\n+world\n*** End Patch",
                "workdir": dir.path()
            }),
        )
        .await
        .expect("add patch should succeed");
    assert!(add_result.content.contains("Applied patch"));
    assert_eq!(
        std::fs::read_to_string(dir.path().join("added.txt")).expect("added file should exist"),
        "hello\nworld"
    );

    registry
        .execute(
            "apply_patch",
            json!({
                "patch": "*** Begin Patch\n*** Update File: sample.txt\n@@\n alpha\n-beta\n+gamma\n*** End Patch",
                "workdir": dir.path()
            }),
        )
        .await
        .expect("update patch should succeed");
    assert_eq!(
        std::fs::read_to_string(&file_path).expect("updated file should exist"),
        "alpha\ngamma\n"
    );

    registry
        .execute(
            "apply_patch",
            json!({
                "patch": "*** Begin Patch\n*** Delete File: added.txt\n*** End Patch",
                "workdir": dir.path()
            }),
        )
        .await
        .expect("delete patch should succeed");
    assert!(!dir.path().join("added.txt").exists());
}

#[tokio::test]
async fn test_apply_patch_context_mismatch_fails() {
    use serde_json::json;
    use tempfile::tempdir;

    let dir = tempdir().expect("tempdir should be created");
    let file_path = dir.path().join("sample.txt");
    std::fs::write(&file_path, "alpha\nbeta\n").expect("seed file should be written");

    let registry = ToolRegistry::new();

    let result = registry
        .execute(
            "apply_patch",
            json!({
                "patch": "*** Begin Patch\n*** Update File: sample.txt\n@@\n alpha\n-missing\n+gamma\n*** End Patch",
                "workdir": dir.path()
            }),
        )
        .await;

    assert!(result.is_err(), "patch should fail when context is missing");
}

#[tokio::test]
async fn test_file_read_range_and_metadata() {
    use serde_json::json;
    use tempfile::tempdir;

    let dir = tempdir().expect("tempdir should be created");
    let file_path = dir.path().join("sample.txt");
    std::fs::write(&file_path, "one\ntwo\nthree\nfour\n").expect("seed file should be written");

    let registry = ToolRegistry::new();
    let result = registry
        .execute(
            "file_read",
            json!({
                "path": file_path,
                "start_line": 2,
                "end_line": 3,
                "max_chars": 1000
            }),
        )
        .await
        .expect("file_read should succeed");

    assert!(result.content.contains("2\ttwo"));
    assert!(result.content.contains("3\tthree"));
    assert_eq!(result.metadata["total_lines"], json!(4));
}

#[tokio::test]
async fn test_grep_and_search_compatibility() {
    use serde_json::json;
    use tempfile::tempdir;

    let dir = tempdir().expect("tempdir should be created");
    let rust_file = dir.path().join("lib.rs");
    let txt_file = dir.path().join("notes.txt");
    std::fs::write(&rust_file, "fn main() {}\nlet value = 1;\n").expect("rust file should exist");
    std::fs::write(&txt_file, "fn fake()\n").expect("txt file should exist");

    let registry = ToolRegistry::new();

    let grep_result = registry
        .execute(
            "grep",
            json!({
                "path": dir.path(),
                "pattern": "fn",
                "include": ["*.rs"],
                "max_results": 10
            }),
        )
        .await
        .expect("grep should succeed");

    assert!(grep_result.content.contains("lib.rs:1"));
    assert!(!grep_result.content.contains("notes.txt"));

    let search_result = registry
        .execute(
            "search",
            json!({
                "path": dir.path(),
                "pattern": "fn",
                "file_pattern": "*.rs",
                "max_results": 10
            }),
        )
        .await
        .expect("search should succeed");

    assert!(search_result.content.contains("lib.rs:1"));
    assert!(!search_result.content.contains("notes.txt"));
}

#[tokio::test]
async fn test_skill_tool_registered() {
    let registry = ToolRegistry::new();
    let tool = registry.get("skill").expect("skill tool should exist");

    assert_eq!(tool.name(), "skill");
    assert!(tool.is_read_only());
    assert!(tool.description().contains("skill"));

    let schema = tool.input_schema();
    assert!(schema["properties"].get("skill").is_some());
    assert!(schema["properties"].get("args").is_some());
}

#[tokio::test]
async fn test_skill_tool_not_configured_error() {
    let registry = ToolRegistry::new();
    let result = registry.execute("skill", serde_json::json!({"skill": "comet"})).await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert_eq!(err.code.as_deref(), Some("skill_registry_unconfigured"));
}

#[tokio::test]
async fn test_glob_tool() {
    use serde_json::json;
    use tempfile::tempdir;

    let dir = tempdir().expect("tempdir should be created");
    std::fs::create_dir_all(dir.path().join("src")).expect("src dir should be created");
    std::fs::write(dir.path().join("src/main.rs"), "fn main() {}").expect("main.rs should exist");
    std::fs::write(dir.path().join("README.md"), "# readme").expect("README should exist");

    let registry = ToolRegistry::new();
    let result = registry
        .execute(
            "glob",
            json!({
                "path": dir.path(),
                "pattern": "*main.rs",
                "max_results": 10
            }),
        )
        .await
        .expect("glob should succeed");

    assert!(result.content.contains("main.rs"));
    assert!(!result.content.contains("README.md"));
}
