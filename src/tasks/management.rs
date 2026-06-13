//! Task Management Tool
//!
//! Manage tasks with operations including:
//! - create: Create a new task
//! - update: Update an existing task
//! - delete: Delete a task
//! - list: List all tasks
//! - complete: Mark a task as completed
//! - set_dependencies: Add/remove dependency relationships
//! - blocked: List all tasks that are blocked by dependencies
//! - get: Get task details

// Re-export types for backward compatibility (e.g., daemon/handlers.rs imports from this module)
pub use super::types::{Task, TaskPriority, TaskStatus};
use crate::tools::{Tool, ToolError, ToolOutput};
use async_trait::async_trait;
use serde_json;
use std::collections::HashMap;
use std::io::Write;
use std::sync::Arc;
use tokio::sync::RwLock;

fn debug_log(msg: &str) {
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/wgenty-code-debug.log")
    {
        let _ = writeln!(f, "{}", msg);
    }
}

pub struct TaskManagementTool {
    tasks: Arc<RwLock<HashMap<String, Task>>>,
}

impl Default for TaskManagementTool {
    fn default() -> Self {
        Self::new()
    }
}

impl TaskManagementTool {
    pub fn new() -> Self {
        Self {
            tasks: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Create a TaskManagementTool that shares the same task store as an existing Arc.
    pub fn from_arc(tasks: Arc<RwLock<HashMap<String, Task>>>) -> Self {
        Self { tasks }
    }

    /// Return the underlying task store so it can be shared.
    pub fn task_store(&self) -> Arc<RwLock<HashMap<String, Task>>> {
        self.tasks.clone()
    }

    /// Return all tasks (excluding deleted ones).
    pub async fn get_all_tasks(&self) -> Vec<Task> {
        let tasks = self.tasks.read().await;
        let total = tasks.len();
        let filtered: Vec<Task> = tasks
            .values()
            .filter(|t| t.status != TaskStatus::Deleted)
            .cloned()
            .collect();
        debug_log(&format!(
            "[get_all_tasks] total={} filtered={} strong_count={}",
            total,
            filtered.len(),
            Arc::strong_count(&self.tasks)
        ));
        filtered
    }

    fn generate_id(&self) -> String {
        uuid::Uuid::new_v4().to_string()
    }

    async fn create_task(&self, input: &serde_json::Value) -> Result<Task, ToolError> {
        let subject = input["subject"].as_str().ok_or_else(|| ToolError {
            message: "subject is required".to_string(),
            code: Some("missing_subject".to_string()),
        })?;

        let description = input["description"].as_str().ok_or_else(|| ToolError {
            message: "description is required".to_string(),
            code: Some("missing_description".to_string()),
        })?;

        let priority = input["priority"]
            .as_str()
            .unwrap_or("medium")
            .parse::<TaskPriority>()
            .map_err(|_| ToolError {
                message: "Invalid priority. Must be low, medium, high, or critical".to_string(),
                code: Some("invalid_priority".to_string()),
            })?;

        let tags: Vec<String> = input["tags"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        let metadata: HashMap<String, serde_json::Value> = input["metadata"]
            .as_object()
            .map(|obj| obj.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
            .unwrap_or_default();

        let blocked_by: Vec<String> = input["blockedBy"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        let now = chrono::Utc::now();
        let task = Task {
            id: self.generate_id(),
            subject: subject.to_string(),
            description: description.to_string(),
            status: TaskStatus::Pending,
            created_at: now,
            updated_at: now,
            metadata,
            tags,
            priority,
            blocked_by,
        };

        Ok(task)
    }

    async fn update_task(
        &self,
        task_id: &str,
        input: &serde_json::Value,
    ) -> Result<Task, ToolError> {
        let mut tasks = self.tasks.write().await;
        let task = tasks.get_mut(task_id).ok_or_else(|| ToolError {
            message: format!("Task not found: {}", task_id),
            code: Some("task_not_found".to_string()),
        })?;

        if let Some(subject) = input["subject"].as_str() {
            task.subject = subject.to_string();
        }

        if let Some(description) = input["description"].as_str() {
            task.description = description.to_string();
        }

        if let Some(status_str) = input["status"].as_str() {
            task.status = status_str.parse::<TaskStatus>().map_err(|_| ToolError {
                message: "Invalid status. Must be pending, in_progress, completed, or deleted"
                    .to_string(),
                code: Some("invalid_status".to_string()),
            })?;
        }

        if let Some(priority_str) = input["priority"].as_str() {
            task.priority = priority_str
                .parse::<TaskPriority>()
                .map_err(|_| ToolError {
                    message: "Invalid priority. Must be low, medium, high, or critical".to_string(),
                    code: Some("invalid_priority".to_string()),
                })?;
        }

        if let Some(tags_array) = input["tags"].as_array() {
            task.tags = tags_array
                .iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect();
        }

        if let Some(metadata_obj) = input["metadata"].as_object() {
            for (key, value) in metadata_obj {
                task.metadata.insert(key.clone(), value.clone());
            }
        }

        task.updated_at = chrono::Utc::now();

        Ok(task.clone())
    }

    /// Check if a task can transition to the given status based on dependency constraints.
    fn can_transition_to(
        &self,
        task: &Task,
        new_status: &TaskStatus,
        all_tasks: &HashMap<String, Task>,
    ) -> Result<(), ToolError> {
        if *new_status == TaskStatus::InProgress || *new_status == TaskStatus::Completed {
            for blocker_id in &task.blocked_by {
                if let Some(blocker) = all_tasks.get(blocker_id) {
                    if blocker.status != TaskStatus::Completed {
                        return Err(ToolError {
                            message: format!(
                                "Cannot set status to {:?}: task is blocked by '{}' (status: {:?})",
                                new_status, blocker.subject, blocker.status
                            ),
                            code: Some("task_blocked".to_string()),
                        });
                    }
                }
            }
        }
        Ok(())
    }
}

#[async_trait]
impl Tool for TaskManagementTool {
    fn name(&self) -> &str {
        "task_management"
    }

    fn description(&self) -> &str {
        "Manage tasks with create, update, delete, list, complete, set_dependencies, and blocked operations"
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "description": "Task operation: create, update, delete, list, complete, get",
                    "enum": ["create", "update", "delete", "list", "complete", "get", "set_dependencies", "blocked"]
                },
                "task_id": {
                    "type": "string",
                    "description": "Task ID (required for update, delete, complete, get)"
                },
                "subject": {
                    "type": "string",
                    "description": "Task subject/title (required for create)"
                },
                "description": {
                    "type": "string",
                    "description": "Task description (required for create)"
                },
                "status": {
                    "type": "string",
                    "description": "Task status: pending, in_progress, completed, deleted",
                    "enum": ["pending", "in_progress", "completed", "deleted"]
                },
                "priority": {
                    "type": "string",
                    "description": "Task priority: low, medium, high, critical",
                    "enum": ["low", "medium", "high", "critical"]
                },
                "tags": {
                    "type": "array",
                    "items": {
                        "type": "string"
                    },
                    "description": "Tags for the task"
                },
                "metadata": {
                    "type": "object",
                    "description": "Additional metadata for the task"
                },
                "blockedBy": {
                    "type": "array",
                    "items": {
                        "type": "string"
                    },
                    "description": "List of task IDs that block this task (used with create and set_dependencies operations)"
                },
                "filter": {
                    "type": "object",
                    "description": "Filter criteria for listing tasks",
                    "properties": {
                        "status": {
                            "type": "string",
                            "description": "Filter by status"
                        },
                        "priority": {
                            "type": "string",
                            "description": "Filter by priority"
                        },
                        "tags": {
                            "type": "array",
                            "items": {
                                "type": "string"
                            },
                            "description": "Filter by tags"
                        }
                    }
                }
            },
            "required": ["operation"]
        })
    }

    async fn execute(&self, input: serde_json::Value) -> Result<ToolOutput, ToolError> {
        let operation = input["operation"].as_str().ok_or_else(|| ToolError {
            message: "operation is required".to_string(),
            code: Some("missing_parameter".to_string()),
        })?;

        debug_log(&format!(
            "[execute] operation={} strong_count={}",
            operation,
            Arc::strong_count(&self.tasks)
        ));

        match operation {
            "create" => self.handle_create(input).await,
            "update" => self.handle_update(input).await,
            "delete" => self.handle_delete(input).await,
            "list" => self.handle_list(input).await,
            "complete" => self.handle_complete(input).await,
            "get" => self.handle_get(input).await,
            "set_dependencies" => self.handle_set_dependencies(input).await,
            "blocked" => self.handle_blocked_tasks().await,
            _ => Err(ToolError {
                message: format!("Unknown task operation: {}", operation),
                code: Some("invalid_operation".to_string()),
            }),
        }
    }
}

impl TaskManagementTool {
    async fn handle_create(&self, input: serde_json::Value) -> Result<ToolOutput, ToolError> {
        let task = self.create_task(&input).await?;
        let task_id = task.id.clone();

        let mut tasks = self.tasks.write().await;
        tasks.insert(task_id.clone(), task);
        let total = tasks.len();
        debug_log(&format!(
            "[handle_create] task_id={} store_total={} strong_count={}",
            task_id,
            total,
            Arc::strong_count(&self.tasks)
        ));

        Ok(ToolOutput {
            output_type: "json".to_string(),
            content: serde_json::json!({
                "success": true,
                "message": "Task created successfully",
                "task_id": task_id
            })
            .to_string(),
            metadata: std::collections::HashMap::new(),
        })
    }

    async fn handle_update(&self, input: serde_json::Value) -> Result<ToolOutput, ToolError> {
        let task_id = input["task_id"].as_str().ok_or_else(|| ToolError {
            message: "task_id is required for update".to_string(),
            code: Some("missing_task_id".to_string()),
        })?;

        // Validate status transition for dependency constraints
        if let Some(status_str) = input["status"].as_str() {
            let new_status = status_str.parse::<TaskStatus>().map_err(|_| ToolError {
                message: "Invalid status. Must be pending, in_progress, completed, or deleted"
                    .to_string(),
                code: Some("invalid_status".to_string()),
            })?;

            let tasks = self.tasks.read().await;
            let task = tasks.get(task_id).ok_or_else(|| ToolError {
                message: format!("Task not found: {}", task_id),
                code: Some("task_not_found".to_string()),
            })?;
            self.can_transition_to(task, &new_status, &tasks)?;
        }

        let task = self.update_task(task_id, &input).await?;

        Ok(ToolOutput {
            output_type: "json".to_string(),
            content: serde_json::json!({
                "success": true,
                "message": "Task updated successfully",
                "task": task
            })
            .to_string(),
            metadata: std::collections::HashMap::new(),
        })
    }

    async fn handle_delete(&self, input: serde_json::Value) -> Result<ToolOutput, ToolError> {
        let task_id = input["task_id"].as_str().ok_or_else(|| ToolError {
            message: "task_id is required for delete".to_string(),
            code: Some("missing_task_id".to_string()),
        })?;

        let mut tasks = self.tasks.write().await;
        let removed = tasks.remove(task_id);

        if removed.is_some() {
            Ok(ToolOutput {
                output_type: "json".to_string(),
                content: serde_json::json!({
                    "success": true,
                    "message": "Task deleted successfully"
                })
                .to_string(),
                metadata: std::collections::HashMap::new(),
            })
        } else {
            Err(ToolError {
                message: format!("Task not found: {}", task_id),
                code: Some("task_not_found".to_string()),
            })
        }
    }

    async fn handle_list(&self, input: serde_json::Value) -> Result<ToolOutput, ToolError> {
        let tasks = self.tasks.read().await;
        debug_log(&format!(
            "[handle_list] store_total={} strong_count={}",
            tasks.len(),
            Arc::strong_count(&self.tasks)
        ));
        let filter = &input["filter"];

        let filtered_tasks: Vec<&Task> = tasks
            .values()
            .filter(|task| {
                if let Some(status_filter) = filter["status"].as_str() {
                    let task_status = match task.status {
                        TaskStatus::Pending => "pending",
                        TaskStatus::InProgress => "in_progress",
                        TaskStatus::Completed => "completed",
                        TaskStatus::Deleted => "deleted",
                    };
                    if task_status != status_filter {
                        return false;
                    }
                }

                if let Some(priority_filter) = filter["priority"].as_str() {
                    let task_priority = match task.priority {
                        TaskPriority::Low => "low",
                        TaskPriority::Medium => "medium",
                        TaskPriority::High => "high",
                        TaskPriority::Critical => "critical",
                    };
                    if task_priority != priority_filter {
                        return false;
                    }
                }

                if let Some(tags_filter) = filter["tags"].as_array() {
                    let filter_tags: Vec<String> = tags_filter
                        .iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect();
                    if !filter_tags.iter().all(|tag| task.tags.contains(tag)) {
                        return false;
                    }
                }

                true
            })
            .collect();

        Ok(ToolOutput {
            output_type: "json".to_string(),
            content: serde_json::json!({
                "success": true,
                "tasks": filtered_tasks,
                "count": filtered_tasks.len()
            })
            .to_string(),
            metadata: std::collections::HashMap::new(),
        })
    }

    async fn handle_complete(&self, input: serde_json::Value) -> Result<ToolOutput, ToolError> {
        let task_id = input["task_id"].as_str().ok_or_else(|| ToolError {
            message: "task_id is required for complete".to_string(),
            code: Some("missing_task_id".to_string()),
        })?;

        let mut tasks = self.tasks.write().await;

        // Clone and check dependencies first
        let task = tasks.get(task_id).cloned().ok_or_else(|| ToolError {
            message: format!("Task not found: {}", task_id),
            code: Some("task_not_found".to_string()),
        })?;

        self.can_transition_to(&task, &TaskStatus::Completed, &tasks)?;

        // Now mutate
        let task = tasks.get_mut(task_id).unwrap();
        task.status = TaskStatus::Completed;
        task.updated_at = chrono::Utc::now();

        Ok(ToolOutput {
            output_type: "json".to_string(),
            content: serde_json::json!({
                "success": true,
                "message": "Task marked as completed",
                "task": task
            })
            .to_string(),
            metadata: std::collections::HashMap::new(),
        })
    }

    async fn handle_get(&self, input: serde_json::Value) -> Result<ToolOutput, ToolError> {
        let task_id = input["task_id"].as_str().ok_or_else(|| ToolError {
            message: "task_id is required for get".to_string(),
            code: Some("missing_task_id".to_string()),
        })?;

        let tasks = self.tasks.read().await;
        let task = tasks.get(task_id).ok_or_else(|| ToolError {
            message: format!("Task not found: {}", task_id),
            code: Some("task_not_found".to_string()),
        })?;

        Ok(ToolOutput {
            output_type: "json".to_string(),
            content: serde_json::json!({
                "success": true,
                "task": task
            })
            .to_string(),
            metadata: std::collections::HashMap::new(),
        })
    }

    async fn handle_set_dependencies(
        &self,
        input: serde_json::Value,
    ) -> Result<ToolOutput, ToolError> {
        let task_id = input["task_id"].as_str().ok_or_else(|| ToolError {
            message: "task_id is required".to_string(),
            code: Some("missing_task_id".to_string()),
        })?;

        let blocked_by: Vec<String> = input["blockedBy"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        let mut tasks = self.tasks.write().await;

        // Validate the target task exists (before mutable borrow)
        if !tasks.contains_key(task_id) {
            return Err(ToolError {
                message: format!("Task not found: {}", task_id),
                code: Some("task_not_found".to_string()),
            });
        }

        // Validate all referenced blocker tasks exist
        for blocker_id in &blocked_by {
            if !tasks.contains_key(blocker_id) {
                return Err(ToolError {
                    message: format!("Blocker task not found: {}", blocker_id),
                    code: Some("blocker_not_found".to_string()),
                });
            }
        }

        // Now mutate
        let task = tasks.get_mut(task_id).unwrap();
        task.blocked_by = blocked_by;
        task.updated_at = chrono::Utc::now();

        Ok(ToolOutput {
            output_type: "json".to_string(),
            content: serde_json::json!({
                "success": true,
                "message": "Dependencies updated",
                "task": task
            })
            .to_string(),
            metadata: std::collections::HashMap::new(),
        })
    }

    async fn handle_blocked_tasks(&self) -> Result<ToolOutput, ToolError> {
        let tasks = self.tasks.read().await;
        let blocked: Vec<&Task> = tasks
            .values()
            .filter(|t| {
                !t.blocked_by.is_empty()
                    && t.status != TaskStatus::Completed
                    && t.status != TaskStatus::Deleted
            })
            .collect();

        Ok(ToolOutput {
            output_type: "json".to_string(),
            content: serde_json::json!({
                "success": true,
                "tasks": blocked,
                "count": blocked.len()
            })
            .to_string(),
            metadata: std::collections::HashMap::new(),
        })
    }
}

#[cfg(test)]
mod tests;
