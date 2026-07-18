//! Checkpoint / Undo tools — per-turn file snapshot facade.
//!
//! These tools sit on top of [`CheckpointStore`]: mutating tools capture
//! pre-edit file content once per turn, and `undo` rewinds only those files
//! without touching unrelated untracked state or running git stash/reset.

use std::sync::Arc;

use crate::tools::checkpoint_store::CheckpointStore;
use crate::tools::{Tool, ToolError, ToolOutput};
use async_trait::async_trait;

/// Thin facade over [`CheckpointStore`] exposed to the model and daemon HTTP.
pub struct CheckpointManager {
    store: Arc<CheckpointStore>,
    /// Last turn that captured a file (or was begun explicitly). Used when
    /// `undo` is called without a turn id.
    last_turn_id: std::sync::Mutex<Option<String>>,
}

impl CheckpointManager {
    pub fn new(store: Arc<CheckpointStore>) -> Self {
        Self {
            store,
            last_turn_id: std::sync::Mutex::new(None),
        }
    }

    pub fn store(&self) -> &Arc<CheckpointStore> {
        &self.store
    }

    /// Remember the active turn so a parameter-less `undo` can target it.
    pub fn note_turn(&self, turn_id: &str) {
        if let Ok(mut g) = self.last_turn_id.lock() {
            *g = Some(turn_id.to_string());
        }
    }

    /// Explicitly open a turn snapshot directory + prune old turns.
    pub fn begin_turn(&self, turn_id: &str) -> Result<(), String> {
        self.store
            .begin_turn(turn_id)
            .map_err(|e| format!("begin_turn failed: {e:#}"))?;
        self.note_turn(turn_id);
        Ok(())
    }

    /// Manual extra snapshot on the current (or provided) turn. Scans the
    /// project for nothing — callers that want a specific file should use the
    /// automatic pre-edit capture path. This records an empty/updated turn so
    /// `list`/`undo` have a handle, and returns the turn id.
    pub async fn create(&self, description: &str) -> Result<String, String> {
        let turn_id = {
            let guard = self
                .last_turn_id
                .lock()
                .map_err(|_| "checkpoint last_turn_id lock poisoned".to_string())?;
            guard
                .clone()
                .unwrap_or_else(|| format!("manual-{}", uuid::Uuid::new_v4()))
        };
        self.begin_turn(&turn_id)?;
        // Touch the manifest description via a note in the summary only — the
        // store has no free-form description field. Returning the turn id is
        // enough for the model to pass back to undo.
        let _ = description;
        Ok(turn_id)
    }

    /// Rewind a turn: restore every file recorded in that turn's manifest.
    ///
    /// `checkpoint_id` is a turn id. When omitted, the most recent turn (by
    /// manifest `created_at`, falling back to the last noted turn) is used.
    pub async fn undo(&self, checkpoint_id: Option<&str>) -> Result<String, String> {
        let turn_id = match checkpoint_id {
            Some(id) if !id.is_empty() => id.to_string(),
            _ => self.resolve_latest_turn_id()?,
        };
        self.store
            .rewind(&turn_id)
            .map_err(|e| format!("rewind failed: {e:#}"))
    }

    /// List recent turn snapshots.
    pub async fn list(&self) -> Result<String, String> {
        let infos = self
            .store
            .list()
            .map_err(|e| format!("list failed: {e:#}"))?;
        if infos.is_empty() {
            return Ok(String::new());
        }
        let mut out = String::new();
        for info in infos {
            out.push_str(&format!(
                "{}  {}  files={}\n",
                info.turn_id, info.created_at, info.file_count
            ));
        }
        Ok(out)
    }

    fn resolve_latest_turn_id(&self) -> Result<String, String> {
        if let Ok(guard) = self.last_turn_id.lock() {
            if let Some(id) = guard.as_ref() {
                return Ok(id.clone());
            }
        }
        let infos = self
            .store
            .list()
            .map_err(|e| format!("list failed: {e:#}"))?;
        infos
            .into_iter()
            .next()
            .map(|i| i.turn_id)
            .ok_or_else(|| "No checkpoint to undo".to_string())
    }
}

pub struct CheckpointTool {
    manager: Arc<CheckpointManager>,
}

impl CheckpointTool {
    pub fn new(manager: Arc<CheckpointManager>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl Tool for CheckpointTool {
    fn name(&self) -> &str {
        "checkpoint"
    }
    fn description(&self) -> &str {
        "Create / open a per-turn file checkpoint. Returns the turn id. File-editing tools automatically capture pre-edit content into the active turn; call this to force a named turn handle for a later undo."
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "description": { "type": "string", "description": "Why this checkpoint is being created" }
            },
            "required": ["description"]
        })
    }
    async fn execute(&self, input: serde_json::Value) -> Result<ToolOutput, ToolError> {
        let desc = input["description"].as_str().unwrap_or("checkpoint");
        match self.manager.create(desc).await {
            Ok(id) => Ok(ToolOutput {
                output_type: "text".to_string(),
                content: format!("Checkpoint created: {id}"),
                metadata: std::collections::HashMap::new(),
            }),
            Err(e) => Err(ToolError {
                message: e,
                code: None,
            }),
        }
    }
}

pub struct UndoTool {
    manager: Arc<CheckpointManager>,
}

impl UndoTool {
    pub fn new(manager: Arc<CheckpointManager>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl Tool for UndoTool {
    fn name(&self) -> &str {
        "undo"
    }
    fn description(&self) -> &str {
        "Rewind files to their pre-edit state for a turn. Pass `checkpoint_id` (the turn id returned by `checkpoint` or emitted by the runtime) to target a specific turn; omit it to restore the latest turn. Only files captured during that turn are touched — unrelated untracked files are left alone. Does not use git stash/reset."
    }
    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "checkpoint_id": {
                    "type": "string",
                    "description": "Optional turn id returned by the `checkpoint` tool or runtime. Targets that specific turn; if omitted, the most recent turn is used."
                }
            }
        })
    }
    async fn execute(&self, input: serde_json::Value) -> Result<ToolOutput, ToolError> {
        let id = input["checkpoint_id"].as_str();
        match self.manager.undo(id).await {
            Ok(output) => Ok(ToolOutput {
                output_type: "text".to_string(),
                content: format!("Checkpoint restored:\n{output}"),
                metadata: std::collections::HashMap::new(),
            }),
            Err(e) => Err(ToolError {
                message: e,
                code: None,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    fn write_file(root: &Path, rel: &str, content: &str) {
        let abs = root.join(rel);
        if let Some(parent) = abs.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(abs, content).unwrap();
    }

    #[tokio::test]
    async fn create_returns_turn_id_and_list_shows_it() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Arc::new(CheckpointStore::new(tmp.path()));
        let mgr = CheckpointManager::new(store);
        let id = mgr.create("before risky op").await.expect("create");
        assert!(!id.is_empty());
        let list = mgr.list().await.expect("list");
        assert!(list.contains(&id), "list should contain {id}: {list}");
    }

    #[tokio::test]
    async fn undo_restores_captured_file_and_preserves_unrelated() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write_file(root, "tracked.txt", "v1");
        write_file(root, "unrelated.txt", "leave me");
        let store = Arc::new(CheckpointStore::new(root));
        let mgr = CheckpointManager::new(store.clone());
        mgr.begin_turn("t1").unwrap();
        store.try_capture_file("t1", "tracked.txt").unwrap();
        write_file(root, "tracked.txt", "v2");
        let summary = mgr.undo(Some("t1")).await.expect("undo");
        assert!(summary.contains("restored"), "{summary}");
        assert_eq!(fs::read_to_string(root.join("tracked.txt")).unwrap(), "v1");
        assert_eq!(
            fs::read_to_string(root.join("unrelated.txt")).unwrap(),
            "leave me"
        );
    }

    #[tokio::test]
    async fn undo_without_id_uses_latest_turn() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write_file(root, "a.txt", "orig");
        let store = Arc::new(CheckpointStore::new(root));
        let mgr = CheckpointManager::new(store.clone());
        mgr.begin_turn("latest").unwrap();
        store.try_capture_file("latest", "a.txt").unwrap();
        write_file(root, "a.txt", "changed");
        mgr.undo(None).await.expect("undo latest");
        assert_eq!(fs::read_to_string(root.join("a.txt")).unwrap(), "orig");
    }

    #[tokio::test]
    async fn undo_unknown_turn_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Arc::new(CheckpointStore::new(tmp.path()));
        let mgr = CheckpointManager::new(store);
        let err = mgr
            .undo(Some("does-not-exist"))
            .await
            .expect_err("unknown turn");
        assert!(
            err.contains("rewind failed") || err.contains("manifest"),
            "unexpected error: {err}"
        );
    }
}
