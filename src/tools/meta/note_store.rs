//! NoteStore — Persistent storage for notes.
//!
//! Storage path: `{dir}/{id}.json`
//! Uses atomic write (tmp → rename) and tokio::fs for all I/O.

use crate::tools::meta::note_edit::Note;
use std::path::{Path, PathBuf};

pub struct NoteStore {
    dir: PathBuf,
}

impl NoteStore {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// Return the storage directory path.
    pub fn path(&self) -> &Path {
        &self.dir
    }

    /// Validate that a note ID is safe for use in filesystem paths.
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

    /// Save a note to disk (atomic write: tmp → rename).
    pub async fn save(&self, note: &Note) -> anyhow::Result<()> {
        if !Self::validate_id(&note.id) {
            anyhow::bail!("Invalid note ID for filesystem path: {:?}", note.id);
        }
        tokio::fs::create_dir_all(&self.dir).await?;

        let file_path = self.file_path(&note.id);
        let content = serde_json::to_string_pretty(note)?;

        let tmp_path = file_path.with_extension("json.tmp");
        tokio::fs::write(&tmp_path, &content).await?;
        tokio::fs::rename(&tmp_path, &file_path).await?;

        Ok(())
    }

    /// Load a note from disk by ID. Returns None if not found.
    pub async fn load(&self, id: &str) -> anyhow::Result<Option<Note>> {
        if !Self::validate_id(id) {
            return Ok(None);
        }
        let file_path = self.file_path(id);
        match tokio::fs::read_to_string(&file_path).await {
            Ok(content) => {
                let note: Note = serde_json::from_str(&content)?;
                Ok(Some(note))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Delete a note from disk by ID.
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

    /// Load all notes from disk.
    pub async fn load_all(&self) -> anyhow::Result<Vec<Note>> {
        if !self.dir.exists() {
            return Ok(Vec::new());
        }

        let mut notes = Vec::new();
        let mut dir = tokio::fs::read_dir(&self.dir).await?;

        while let Some(entry) = dir.next_entry().await? {
            let path = entry.path();
            if path.extension().map(|e| e == "json").unwrap_or(false) {
                if let Ok(content) = tokio::fs::read_to_string(&path).await {
                    if let Ok(note) = serde_json::from_str::<Note>(&content) {
                        notes.push(note);
                    }
                }
            }
        }

        Ok(notes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::meta::note_edit::NoteFormat;
    use std::collections::HashMap;

    fn make_note(id: &str, title: &str, content: &str) -> Note {
        Note {
            id: id.to_string(),
            title: title.to_string(),
            content: content.to_string(),
            format: NoteFormat::Markdown,
            tags: vec!["test".to_string()],
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            metadata: HashMap::new(),
        }
    }

    #[tokio::test]
    async fn test_save_and_load_note() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = NoteStore::new(tmp.path().to_path_buf());

        let note = make_note("note-1", "Test Note", "Hello world");
        store.save(&note).await.unwrap();

        let loaded = store.load("note-1").await.unwrap();
        assert!(loaded.is_some());
        let loaded = loaded.unwrap();
        assert_eq!(loaded.id, "note-1");
        assert_eq!(loaded.title, "Test Note");
        assert_eq!(loaded.content, "Hello world");
    }

    #[tokio::test]
    async fn test_load_nonexistent_returns_none() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = NoteStore::new(tmp.path().to_path_buf());

        let loaded = store.load("nonexistent").await.unwrap();
        assert!(loaded.is_none());
    }

    #[tokio::test]
    async fn test_delete_note() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = NoteStore::new(tmp.path().to_path_buf());

        let note = make_note("note-del", "Delete Me", "bye");
        store.save(&note).await.unwrap();

        // Verify it exists
        let loaded = store.load("note-del").await.unwrap();
        assert!(loaded.is_some());

        // Delete it
        store.delete("note-del").await.unwrap();

        // Verify it's gone
        let loaded = store.load("note-del").await.unwrap();
        assert!(loaded.is_none());
    }

    #[tokio::test]
    async fn test_delete_nonexistent_is_noop() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = NoteStore::new(tmp.path().to_path_buf());

        // Should not error
        store.delete("nonexistent").await.unwrap();
    }

    #[tokio::test]
    async fn test_load_all_notes() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = NoteStore::new(tmp.path().to_path_buf());

        let note1 = make_note("note-a", "A", "Content A");
        let note2 = make_note("note-b", "B", "Content B");
        store.save(&note1).await.unwrap();
        store.save(&note2).await.unwrap();

        let all = store.load_all().await.unwrap();
        assert_eq!(all.len(), 2);

        let ids: Vec<&str> = all.iter().map(|n| n.id.as_str()).collect();
        assert!(ids.contains(&"note-a"));
        assert!(ids.contains(&"note-b"));
    }

    #[tokio::test]
    async fn test_load_all_empty_dir() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = NoteStore::new(tmp.path().to_path_buf());

        let all = store.load_all().await.unwrap();
        assert!(all.is_empty());
    }

    #[tokio::test]
    async fn test_save_updates_existing_note() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = NoteStore::new(tmp.path().to_path_buf());

        let note = make_note("note-update", "Original", "Original content");
        store.save(&note).await.unwrap();

        let updated = Note {
            title: "Updated".to_string(),
            content: "Updated content".to_string(),
            ..note.clone()
        };
        store.save(&updated).await.unwrap();

        let loaded = store.load("note-update").await.unwrap().unwrap();
        assert_eq!(loaded.title, "Updated");
        assert_eq!(loaded.content, "Updated content");
    }

    #[tokio::test]
    async fn test_auto_creates_directory() {
        let tmp = tempfile::TempDir::new().unwrap();
        let nested = tmp.path().join("sub").join("notes");
        let store = NoteStore::new(nested.clone());

        assert!(!nested.exists());

        let note = make_note("note-dir", "Dir Test", "content");
        store.save(&note).await.unwrap();

        assert!(nested.exists());
        let loaded = store.load("note-dir").await.unwrap();
        assert!(loaded.is_some());
    }

    #[tokio::test]
    async fn test_invalid_id_rejected_on_save() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = NoteStore::new(tmp.path().to_path_buf());

        let note = make_note("../../../etc/passwd", "bad", "bad");
        let result = store.save(&note).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_invalid_id_returns_none_on_load() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = NoteStore::new(tmp.path().to_path_buf());

        let loaded = store.load("../../../etc/passwd").await.unwrap();
        assert!(loaded.is_none());
    }

    #[tokio::test]
    async fn test_invalid_id_is_noop_on_delete() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = NoteStore::new(tmp.path().to_path_buf());

        store.delete("../../../etc/passwd").await.unwrap();
        // Should not panic/error
    }
}
