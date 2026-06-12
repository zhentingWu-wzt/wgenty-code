use super::*;
use crate::tools::Tool;

#[tokio::test]
async fn test_shared_task_store() {
    // Simulate DaemonState initialization
    let task_manager = Arc::new(TaskManagementTool::new());
    let shared_store = task_manager.task_store();
    let tool = TaskManagementTool::from_arc(shared_store);

    // Create a task via the tool (simulates agent calling task_management)
    let input = serde_json::json!({
        "operation": "create",
        "subject": "test task",
        "description": "verify shared store",
        "priority": "high"
    });
    let result = tool.execute(input).await.unwrap();
    assert!(result.content.contains("success"));

    // Extract task_id for dependency test
    let data: serde_json::Value = serde_json::from_str(&result.content).unwrap();
    let task_a_id = data["task_id"].as_str().unwrap().to_string();

    // Verify task_manager sees the task
    let all = task_manager.get_all_tasks().await;
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].subject, "test task");
    assert_eq!(all[0].status, TaskStatus::Pending);
    assert_eq!(all[0].priority, TaskPriority::High);

    // Create another task via task_manager
    let input2 = serde_json::json!({
        "operation": "create",
        "subject": "second task",
        "description": "another one"
    });
    task_manager.execute(input2).await.unwrap();

    // Verify tool sees both tasks
    let result = tool
        .execute(serde_json::json!({"operation": "list"}))
        .await
        .unwrap();
    let data: serde_json::Value = serde_json::from_str(&result.content).unwrap();
    assert_eq!(data["count"].as_u64().unwrap(), 2);

    // Test task dependencies (blockedBy)
    // Create a task with blockedBy referencing the first task
    let result_b = tool
        .execute(serde_json::json!({
            "operation": "create",
            "subject": "blocked task",
            "description": "depends on first task",
            "blockedBy": [task_a_id]
        }))
        .await
        .unwrap();
    let data_b: serde_json::Value = serde_json::from_str(&result_b.content).unwrap();
    let task_b_id = data_b["task_id"].as_str().unwrap().to_string();

    // Try to complete B before A -> should fail
    let err = tool
        .execute(serde_json::json!({
            "operation": "complete",
            "task_id": task_b_id
        }))
        .await
        .unwrap_err();
    assert!(
        err.message.contains("blocked by"),
        "Expected blocked by error, got: {}",
        err.message
    );

    // Try to set B to in_progress before A is completed -> should fail
    let err = tool
        .execute(serde_json::json!({
            "operation": "update",
            "task_id": task_b_id,
            "status": "in_progress"
        }))
        .await
        .unwrap_err();
    assert!(
        err.message.contains("blocked by"),
        "Expected blocked by error, got: {}",
        err.message
    );

    // Complete A first
    tool.execute(serde_json::json!({
        "operation": "complete",
        "task_id": task_a_id
    }))
    .await
    .unwrap();

    // Now complete B -> should succeed
    let result = tool
        .execute(serde_json::json!({
            "operation": "complete",
            "task_id": task_b_id
        }))
        .await
        .unwrap();
    assert!(
        result.content.contains("success"),
        "Expected success completing B after A is done"
    );

    // Test blocked operation — all tasks completed, so no blocked tasks
    let blocked_result = tool
        .execute(serde_json::json!({
            "operation": "blocked"
        }))
        .await
        .unwrap();
    let blocked_data: serde_json::Value =
        serde_json::from_str(&blocked_result.content).unwrap();
    assert_eq!(blocked_data["count"].as_u64().unwrap(), 0);

    // Test set_dependencies operation with invalid blocker
    let result_c = tool
        .execute(serde_json::json!({
            "operation": "create",
            "subject": "task C",
            "description": "will be blocked"
        }))
        .await
        .unwrap();
    let data_c: serde_json::Value = serde_json::from_str(&result_c.content).unwrap();
    let task_c_id = data_c["task_id"].as_str().unwrap().to_string();

    // Invalid blocker should fail
    let err = tool
        .execute(serde_json::json!({
            "operation": "set_dependencies",
            "task_id": task_c_id,
            "blockedBy": ["nonexistent-id"]
        }))
        .await
        .unwrap_err();
    assert!(
        err.message.contains("Blocker task not found"),
        "Expected blocker not found error"
    );

    // Set valid dependencies
    let result = tool
        .execute(serde_json::json!({
            "operation": "set_dependencies",
            "task_id": task_c_id,
            "blockedBy": [task_a_id]
        }))
        .await
        .unwrap();
    assert!(
        result.content.contains("Dependencies updated"),
        "Expected dependencies updated"
    );
}
