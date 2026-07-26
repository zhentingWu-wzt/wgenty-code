use wgenty_code::tools::ToolRegistry;

#[tokio::test]
async fn test_new_tools_functionality() {
    let registry = ToolRegistry::new();
    let tool_names: Vec<String> = registry
        .list()
        .iter()
        .map(|tool| tool.name().to_string())
        .collect();

    assert!(tool_names.iter().any(|n| n == "file_read"));
    assert!(tool_names.iter().any(|n| n == "file_edit"));
    assert!(tool_names.iter().any(|n| n == "file_write"));
    assert!(tool_names.iter().any(|n| n == "list_files"));
    assert!(tool_names.iter().any(|n| n == "view"));
    assert!(tool_names.iter().any(|n| n == "apply_patch"));
    assert!(tool_names.iter().any(|n| n == "execute_command"));
    assert!(tool_names.iter().any(|n| n == "exec_command"));
    assert!(tool_names.iter().any(|n| n == "kill_session"));
    assert!(tool_names.iter().any(|n| n == "git_operations"));
    assert!(tool_names.iter().any(|n| n == "run_test"));
    assert!(tool_names.iter().any(|n| n == "search"));
    assert!(tool_names.iter().any(|n| n == "grep"));
    assert!(tool_names.iter().any(|n| n == "glob"));
    assert!(tool_names.iter().any(|n| n == "web_search"));
    assert!(tool_names.iter().any(|n| n == "web_fetch"));
    assert!(tool_names.iter().any(|n| n == "ask_user_question"));
    assert!(tool_names.iter().any(|n| n == "update_plan"));
    assert!(tool_names.iter().any(|n| n == "think"));
    assert!(tool_names.iter().any(|n| n == "compact"));
    assert!(tool_names.iter().any(|n| n == "lsp"));
    assert!(tool_names.iter().any(|n| n == "checkpoint"));
    assert!(tool_names.iter().any(|n| n == "undo"));
    assert!(tool_names.iter().any(|n| n == "task_management"));
}

#[tokio::test]
async fn test_tool_system_integration() {
    use serde_json::json;

    let registry = ToolRegistry::new();

    let read_result = registry
        .execute(
            "file_read",
            json!({"path": "Cargo.toml", "start_line": 1, "end_line": 5}),
        )
        .await;

    assert!(read_result.is_ok());

    let search_result = registry
        .execute("search", json!({"path": ".", "pattern": "wgenty_code"}))
        .await;

    assert!(search_result.is_ok());

    let git_result = registry
        .execute("git_operations", json!({"operation": "status"}))
        .await;

    let _ = git_result;
}
