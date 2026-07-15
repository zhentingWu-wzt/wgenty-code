//! TaskStore — Persistent storage for tasks.
//!
//! Storage path: `{dir}/{id}.json`
//! Uses atomic write (tmp → rename) and tokio::fs for all I/O.

use crate::tasks::types::Task;
use std::path::{Path, PathBuf};

pub struct TaskStore {
    dir: PathBuf,
}

impl TaskStore {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// Return the storage directory path.
    pub fn path(&self) -> &Path {
        &self.dir
    }

    /// Validate that a task ID is safe for use in filesystem paths.
    fn validate_id(id: &str) -> bool {
        if id.is_empty() || id.len() > 255 {
            return false;
        }
        if id.contains('/') || id.contains('\\') || id.contains("..") {
            return false;
        }
        if id.contains('\0') || id.starts_with('.') {
            return false;
        }
        // Reject Windows reserved names
        let upper = id.to_uppercase();
        let stem = upper.split('.').next().unwrap_or(&upper);
        const RESERVED: &[&str] = &[
            "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
            "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
        ];
        if RESERVED.contains(&stem) {
            return false;
        }
        true
    }

    fn file_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{}.json", id))
    }

    /// Save a task to disk (atomic write: tmp → rename).
    pub async fn save(&self, task: &Task) -> anyhow::Result<()> {
        if !Self::validate_id(&task.id) {
            anyhow::bail!("Invalid task ID for filesystem path: {:?}", task.id);
        }
        tokio::fs::create_dir_all(&self.dir).await?;

        let file_path = self.file_path(&task.id);
        let content = serde_json::to_string_pretty(task)?;

        let tmp_path = file_path.with_extension("json.tmp");
        tokio::fs::write(&tmp_path, &content).await?;
        tokio::fs::rename(&tmp_path, &file_path).await?;

        Ok(())
    }

    /// Load a task from disk by ID. Returns None if not found.
    pub async fn load(&self, id: &str) -> anyhow::Result<Option<Task>> {
        if !Self::validate_id(id) {
            return Ok(None);
        }
        let file_path = self.file_path(id);
        match tokio::fs::read_to_string(&file_path).await {
            Ok(content) => {
                let task: Task = serde_json::from_str(&content)?;
                Ok(Some(task))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Delete a task from disk by ID.
    pub async fn delete(&self, id: &str) -> anyhow::Result<()> {
        if !Self::validate_id(id) {
            return Ok(());
        }
        let file_path = self.file_path(id);
        match tokio::fs::remove_file(&file_path).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }

    /// Load all tasks from disk.
    pub async fn load_all(&self) -> anyhow::Result<Vec<Task>> {
        if !self.dir.exists() {
            return Ok(Vec::new());
        }

        let mut tasks = Vec::new();
        let mut dir = tokio::fs::read_dir(&self.dir).await?;

        while let Some(entry) = dir.next_entry().await? {
            let path = entry.path();
            if path.extension().map(|e| e == "json").unwrap_or(false) {
                if let Ok(content) = tokio::fs::read_to_string(&path).await {
                    if let Ok(task) = serde_json::from_str::<Task>(&content) {
                        tasks.push(task);
                    }
                }
            }
        }

        Ok(tasks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::types::{TaskPriority, TaskStatus};
    use std::collections::HashMap;

    fn make_task(id: &str, subject: &str, description: &str) -> Task {
        Task {
            id: id.to_string(),
            subject: subject.to_string(),
            description: description.to_string(),
            status: TaskStatus::Pending,
            priority: TaskPriority::Medium,
            tags: vec!["test".to_string()],
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            metadata: HashMap::new(),
            blocked_by: Vec::new(),
        }
    }

    #[tokio::test]
    async fn test_save_and_load_task() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = TaskStore::new(tmp.path().to_path_buf());

        let task = make_task("task-1", "Test Task", "Do something");
        store.save(&task).await.unwrap();

        let loaded = store.load("task-1").await.unwrap();
        assert!(loaded.is_some());
        let loaded = loaded.unwrap();
        assert_eq!(loaded.id, "task-1");
        assert_eq!(loaded.subject, "Test Task");
        assert_eq!(loaded.description, "Do something");
        assert_eq!(loaded.status, TaskStatus::Pending);
    }

    #[tokio::test]
    async fn test_load_nonexistent_returns_none() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = TaskStore::new(tmp.path().to_path_buf());

        let loaded = store.load("nonexistent").await.unwrap();
        assert!(loaded.is_none());
    }

    #[tokio::test]
    async fn test_delete_task() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = TaskStore::new(tmp.path().to_path_buf());

        let task = make_task("task-del", "Delete Me", "bye");
        store.save(&task).await.unwrap();

        let loaded = store.load("task-del").await.unwrap();
        assert!(loaded.is_some());

        store.delete("task-del").await.unwrap();

        let loaded = store.load("task-del").await.unwrap();
        assert!(loaded.is_none());
    }

    #[tokio::test]
    async fn test_delete_nonexistent_is_noop() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = TaskStore::new(tmp.path().to_path_buf());

        store.delete("nonexistent").await.unwrap();
    }

    #[tokio::test]
    async fn test_load_all_tasks() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = TaskStore::new(tmp.path().to_path_buf());

        let task1 = make_task("task-a", "A", "Content A");
        let task2 = make_task("task-b", "B", "Content B");
        store.save(&task1).await.unwrap();
        store.save(&task2).await.unwrap();

        let all = store.load_all().await.unwrap();
        assert_eq!(all.len(), 2);

        let ids: Vec<&str> = all.iter().map(|t| t.id.as_str()).collect();
        assert!(ids.contains(&"task-a"));
        assert!(ids.contains(&"task-b"));
    }

    #[tokio::test]
    async fn test_load_all_empty_dir() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = TaskStore::new(tmp.path().to_path_buf());

        let all = store.load_all().await.unwrap();
        assert!(all.is_empty());
    }

    #[tokio::test]
    async fn test_save_updates_existing_task() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = TaskStore::new(tmp.path().to_path_buf());

        let task = make_task("task-update", "Original", "Original desc");
        store.save(&task).await.unwrap();

        let mut updated = task.clone();
        updated.subject = "Updated".to_string();
        updated.status = TaskStatus::Completed;
        store.save(&updated).await.unwrap();

        let loaded = store.load("task-update").await.unwrap().unwrap();
        assert_eq!(loaded.subject, "Updated");
        assert_eq!(loaded.status, TaskStatus::Completed);
    }

    #[tokio::test]
    async fn test_auto_creates_directory() {
        let tmp = tempfile::TempDir::new().unwrap();
        let nested = tmp.path().join("sub").join("tasks");
        let store = TaskStore::new(nested.clone());

        assert!(!nested.exists());

        let task = make_task("task-dir", "Dir Test", "content");
        store.save(&task).await.unwrap();

        assert!(nested.exists());
        let loaded = store.load("task-dir").await.unwrap();
        assert!(loaded.is_some());
    }

    #[tokio::test]
    async fn test_invalid_id_rejected_on_save() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = TaskStore::new(tmp.path().to_path_buf());

        let task = make_task("../../../etc/passwd", "bad", "bad");
        let result = store.save(&task).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_invalid_id_returns_none_on_load() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = TaskStore::new(tmp.path().to_path_buf());

        let loaded = store.load("../../../etc/passwd").await.unwrap();
        assert!(loaded.is_none());
    }

    #[tokio::test]
    async fn test_invalid_id_is_noop_on_delete() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = TaskStore::new(tmp.path().to_path_buf());

        store.delete("../../../etc/passwd").await.unwrap();
    }

    #[tokio::test]
    async fn test_blocked_by_round_trip() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = TaskStore::new(tmp.path().to_path_buf());

        let mut task = make_task("task-dep", "Dep", "has blockers");
        task.blocked_by = vec!["blocker-1".to_string(), "blocker-2".to_string()];
        store.save(&task).await.unwrap();

        let loaded = store.load("task-dep").await.unwrap().unwrap();
        assert_eq!(
            loaded.blocked_by,
            vec!["blocker-1".to_string(), "blocker-2".to_string()]
        );
    }
}
