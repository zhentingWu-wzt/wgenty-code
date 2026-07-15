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
    let blocked_data: serde_json::Value = serde_json::from_str(&blocked_result.content).unwrap();
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

#[tokio::test]
async fn test_dependency_cycle_rejected() {
    let tool = TaskManagementTool::new();

    let a = tool
        .execute(serde_json::json!({
            "operation": "create",
            "subject": "A",
            "description": "root"
        }))
        .await
        .unwrap();
    let a_id = serde_json::from_str::<serde_json::Value>(&a.content).unwrap()["task_id"]
        .as_str()
        .unwrap()
        .to_string();

    let b = tool
        .execute(serde_json::json!({
            "operation": "create",
            "subject": "B",
            "description": "depends on A",
            "blockedBy": [a_id]
        }))
        .await
        .unwrap();
    let b_id = serde_json::from_str::<serde_json::Value>(&b.content).unwrap()["task_id"]
        .as_str()
        .unwrap()
        .to_string();

    // A blocked_by B would cycle A→B→A
    let err = tool
        .execute(serde_json::json!({
            "operation": "set_dependencies",
            "task_id": a_id,
            "blockedBy": [b_id]
        }))
        .await
        .unwrap_err();
    assert_eq!(err.code.as_deref(), Some("dependency_cycle"));
    assert!(err.message.contains("cycle"), "got: {}", err.message);
}

#[tokio::test]
async fn test_create_rejects_missing_blocker() {
    let tool = TaskManagementTool::new();
    let err = tool
        .execute(serde_json::json!({
            "operation": "create",
            "subject": "orphan dep",
            "description": "bad",
            "blockedBy": ["does-not-exist"]
        }))
        .await
        .unwrap_err();
    assert_eq!(err.code.as_deref(), Some("blocker_not_found"));
}

#[tokio::test]
async fn test_ready_and_blocked_operations() {
    let tool = TaskManagementTool::new();

    let a = tool
        .execute(serde_json::json!({
            "operation": "create",
            "subject": "A",
            "description": "blocker"
        }))
        .await
        .unwrap();
    let a_id = serde_json::from_str::<serde_json::Value>(&a.content).unwrap()["task_id"]
        .as_str()
        .unwrap()
        .to_string();

    let b = tool
        .execute(serde_json::json!({
            "operation": "create",
            "subject": "B",
            "description": "blocked",
            "blockedBy": [a_id]
        }))
        .await
        .unwrap();
    let b_id = serde_json::from_str::<serde_json::Value>(&b.content).unwrap()["task_id"]
        .as_str()
        .unwrap()
        .to_string();

    let ready = tool
        .execute(serde_json::json!({"operation": "ready"}))
        .await
        .unwrap();
    let ready_data: serde_json::Value = serde_json::from_str(&ready.content).unwrap();
    assert_eq!(ready_data["count"].as_u64().unwrap(), 1);
    assert_eq!(
        ready_data["tasks"][0]["id"].as_str().unwrap(),
        a_id.as_str()
    );

    let blocked = tool
        .execute(serde_json::json!({"operation": "blocked"}))
        .await
        .unwrap();
    let blocked_data: serde_json::Value = serde_json::from_str(&blocked.content).unwrap();
    assert_eq!(blocked_data["count"].as_u64().unwrap(), 1);
    assert_eq!(
        blocked_data["tasks"][0]["task"]["id"].as_str().unwrap(),
        b_id.as_str()
    );
    assert_eq!(
        blocked_data["tasks"][0]["open_blockers"][0]
            .as_str()
            .unwrap(),
        a_id.as_str()
    );

    // Complete A → B becomes ready
    tool.execute(serde_json::json!({
        "operation": "complete",
        "task_id": a_id
    }))
    .await
    .unwrap();

    let ready2 = tool
        .execute(serde_json::json!({"operation": "ready"}))
        .await
        .unwrap();
    let ready2_data: serde_json::Value = serde_json::from_str(&ready2.content).unwrap();
    let ready_ids: Vec<&str> = ready2_data["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t["id"].as_str())
        .collect();
    assert!(ready_ids.contains(&b_id.as_str()));

    let blocked2 = tool
        .execute(serde_json::json!({"operation": "blocked"}))
        .await
        .unwrap();
    let blocked2_data: serde_json::Value = serde_json::from_str(&blocked2.content).unwrap();
    assert_eq!(blocked2_data["count"].as_u64().unwrap(), 0);
}

#[tokio::test]
async fn test_self_dependency_rejected() {
    let tool = TaskManagementTool::new();
    let created = tool
        .execute(serde_json::json!({
            "operation": "create",
            "subject": "solo",
            "description": "x"
        }))
        .await
        .unwrap();
    let id = serde_json::from_str::<serde_json::Value>(&created.content).unwrap()["task_id"]
        .as_str()
        .unwrap()
        .to_string();

    let err = tool
        .execute(serde_json::json!({
            "operation": "set_dependencies",
            "task_id": id,
            "blockedBy": [id]
        }))
        .await
        .unwrap_err();
    assert_eq!(err.code.as_deref(), Some("dependency_cycle"));
}
