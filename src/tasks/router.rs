//! Per-project routing for [`TaskManagementTool`] and [`TodoState`] (s03).
//!
//! Mirrors `daemon::memory_router::MemoryRouter`: the daemon's main project
//! keeps the instance built at startup; every other registered project
//! lazily gets its own instance backed by `<project>/.wgenty-code/tasks/`.
//! The daemon resolves the project root via `effective_session_root` and
//! calls `for_project(root)`; the router itself never depends on
//! `daemon::projects` to keep `tasks/` free of daemon-layer coupling.

use crate::tasks::management::TaskManagementTool;
use crate::tasks::store::TaskStore;
use crate::tasks::todo_write::TodoState;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Routes task operations to per-project [`TaskManagementTool`] instances.
///
/// The main project reuses the shared instance (so HTTP handlers that read
/// the "default" task list still see agent-created tasks). Additional
/// projects get lazy-initialized instances with persistence at
/// `<project>/.wgenty-code/tasks/`.
pub struct TaskRouter {
    main: Arc<TaskManagementTool>,
    main_root: PathBuf,
    managers: RwLock<HashMap<PathBuf, Arc<TaskManagementTool>>>,
}

impl TaskRouter {
    pub fn new(main: Arc<TaskManagementTool>, main_root: PathBuf) -> Self {
        Self {
            main,
            main_root,
            managers: RwLock::new(HashMap::new()),
        }
    }

    /// The main project's task manager (built at daemon startup).
    pub fn main(&self) -> Arc<TaskManagementTool> {
        self.main.clone()
    }

    /// Task manager for an exact project root (get-or-create).
    ///
    /// Main project → shared instance; other projects → lazy-created with
    /// a [`TaskStore`] rooted at `<root>/.wgenty-code/tasks/`.
    pub async fn for_project(&self, root: &Path) -> Arc<TaskManagementTool> {
        if root == self.main_root {
            return self.main.clone();
        }
        if let Some(m) = self.managers.read().await.get(root) {
            return m.clone();
        }
        let tasks_dir = root.join(".wgenty-code").join("tasks");
        let store = TaskStore::new(tasks_dir);
        let tool = Arc::new(TaskManagementTool::new_with_store(store));
        let _ = tool.load_from_store().await;
        // Double-check: a concurrent creator may have won the race.
        self.managers
            .write()
            .await
            .entry(root.to_path_buf())
            .or_insert(tool)
            .clone()
    }
}

/// Routes todo state to per-project [`TodoState`] instances.
///
/// `TodoState` is in-memory only (no persistence), so non-main projects
/// get a fresh `TodoState::default()` on first access.
pub struct TodoRouter {
    main: Arc<RwLock<TodoState>>,
    main_root: PathBuf,
    managers: RwLock<HashMap<PathBuf, Arc<RwLock<TodoState>>>>,
}

impl TodoRouter {
    pub fn new(main: Arc<RwLock<TodoState>>, main_root: PathBuf) -> Self {
        Self {
            main,
            main_root,
            managers: RwLock::new(HashMap::new()),
        }
    }

    /// The main project's todo state (built at daemon startup).
    pub fn main(&self) -> Arc<RwLock<TodoState>> {
        self.main.clone()
    }

    /// Todo state for an exact project root (get-or-create).
    pub async fn for_project(&self, root: &Path) -> Arc<RwLock<TodoState>> {
        if root == self.main_root {
            return self.main.clone();
        }
        self.managers
            .write()
            .await
            .entry(root.to_path_buf())
            .or_insert_with(|| Arc::new(RwLock::new(TodoState::default())))
            .clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn task_router_main_reuses_shared_instance() {
        let main = Arc::new(TaskManagementTool::new());
        let router = TaskRouter::new(main.clone(), PathBuf::from("/main"));

        let got = router.for_project(Path::new("/main")).await;
        assert!(Arc::ptr_eq(&got, &main));
    }

    #[tokio::test]
    async fn task_router_other_project_creates_and_caches() {
        let tmp = tempfile::tempdir().unwrap();
        let main = Arc::new(TaskManagementTool::new());
        let router = TaskRouter::new(main, PathBuf::from("/main"));

        let root = tmp.path().canonicalize().unwrap();
        let first = router.for_project(&root).await;
        let second = router.for_project(&root).await;
        assert!(Arc::ptr_eq(&first, &second));
    }

    #[tokio::test]
    async fn todo_router_main_reuses_shared_instance() {
        let main = Arc::new(RwLock::new(TodoState::default()));
        let router = TodoRouter::new(main.clone(), PathBuf::from("/main"));

        let got = router.for_project(Path::new("/main")).await;
        assert!(Arc::ptr_eq(&got, &main));
    }

    #[tokio::test]
    async fn todo_router_other_project_isolates_state() {
        let main = Arc::new(RwLock::new(TodoState::default()));
        let router = TodoRouter::new(main, PathBuf::from("/main"));

        let other = router.for_project(Path::new("/other")).await;
        other.write().await.items.push(crate::tasks::TodoItem {
            content: "isolated".into(),
            status: "pending".into(),
            active_form: String::new(),
            subagent: None,
        });

        // Main project's todo state is unaffected.
        let main_state = router.for_project(Path::new("/main")).await;
        assert!(main_state.read().await.items.is_empty());
    }
}
