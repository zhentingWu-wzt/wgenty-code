//! Task board, todos and background result handlers.

use super::*;

pub async fn list_tasks(
    State(state): State<Arc<DaemonState>>,
    Query(params): Query<HashMap<String, String>>,
) -> Json<ListTasksResponse> {
    let task_manager = match params.get("session_id") {
        Some(sid) => state.task_manager_for_session(sid).await,
        None => state.task_router.main(),
    };
    let all = task_manager.get_all_tasks().await;
    debug_log(&format!(
        "[list_tasks handler] returning {} tasks",
        all.len()
    ));
    let tasks: Vec<TaskInfo> = all
        .into_iter()
        .map(|t| TaskInfo {
            id: t.id,
            subject: t.subject,
            description: t.description,
            status: match t.status {
                TaskStatus::Pending => "pending",
                TaskStatus::InProgress => "in_progress",
                TaskStatus::Completed => "completed",
                TaskStatus::Deleted => "deleted",
            }
            .to_string(),
            priority: match t.priority {
                TaskPriority::Low => "low",
                TaskPriority::Medium => "medium",
                TaskPriority::High => "high",
                TaskPriority::Critical => "critical",
            }
            .to_string(),
            created_at: t.created_at.to_rfc3339(),
            updated_at: t.updated_at.to_rfc3339(),
            tags: t.tags,
        })
        .collect();

    Json(ListTasksResponse { tasks })
}

/// `GET /api/v1/tasks/progress` - ready/blocked counts for agent-loop nudges.
pub async fn task_progress(
    State(state): State<Arc<DaemonState>>,
    Query(params): Query<HashMap<String, String>>,
) -> Json<crate::daemon::models::TaskProgressResponse> {
    let task_manager = match params.get("session_id") {
        Some(sid) => state.task_manager_for_session(sid).await,
        None => state.task_router.main(),
    };
    let store = task_manager.task_store();
    let map = store.read().await;
    let all: std::collections::HashMap<String, crate::tasks::Task> = map.clone();
    drop(map);
    let blocked = crate::tasks::blocked_tasks(&all).len();
    let ready = crate::tasks::ready_tasks(&all).len();
    Json(crate::daemon::models::TaskProgressResponse { blocked, ready })
}

// ── Todos (s03 TodoWrite) ────────────────────────────────────────────────────

pub async fn get_todos(
    State(state): State<Arc<DaemonState>>,
    Query(params): Query<HashMap<String, String>>,
) -> Json<GetTodosResponse> {
    let todo_state = match params.get("session_id") {
        Some(sid) => state.todo_state_for_session(sid).await,
        None => state.todo_router.main(),
    };
    let todo_state = todo_state.read().await;
    let items: Vec<TodoItemResponse> = todo_state
        .items
        .iter()
        .map(|t| TodoItemResponse {
            content: t.content.clone(),
            status: t.status.clone(),
            active_form: t.active_form.clone(),
            subagent: t.subagent.clone(),
        })
        .collect();
    let has_open = todo_state.has_open_items();
    let display = todo_state.render();
    Json(GetTodosResponse {
        items,
        has_open_items: has_open,
        display,
    })
}

// ── Background Tasks ──────────────────────────────────────────────────────────

pub async fn get_background_results(
    State(state): State<Arc<DaemonState>>,
) -> Json<serde_json::Value> {
    // Snapshot read (no drain): results are retained so every client can
    // query them; the old first-come-first-served drain is abolished.
    let results = state.background_results_snapshot().await;
    Json(serde_json::json!({ "results": results }))
}

// ── Subagent Progress ────────────────────────────────────────────────────────

// ── MCP ──────────────────────────────────────────────────────────────────────
