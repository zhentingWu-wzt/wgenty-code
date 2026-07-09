//! History Management - Command and query history

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub id: String,
    pub entry_type: HistoryType,
    pub content: String,
    pub timestamp: DateTime<Utc>,
    pub session_id: Option<String>,
    pub success: bool,
    pub duration_ms: Option<u64>,
    pub metadata: serde_json::Value,
}

impl HistoryEntry {
    pub fn new(entry_type: HistoryType, content: &str) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            entry_type,
            content: content.to_string(),
            timestamp: Utc::now(),
            session_id: None,
            success: true,
            duration_ms: None,
            metadata: serde_json::Value::Null,
        }
    }

    pub fn with_session(mut self, session_id: &str) -> Self {
        self.session_id = Some(session_id.to_string());
        self
    }

    pub fn with_duration(mut self, duration_ms: u64) -> Self {
        self.duration_ms = Some(duration_ms);
        self
    }

    pub fn with_success(mut self, success: bool) -> Self {
        self.success = success;
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum HistoryType {
    Command,
    Query,
    ToolCall,
    FileOperation,
    Search,
    Agent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryFilter {
    pub entry_type: Option<HistoryType>,
    pub session_id: Option<String>,
    pub success_only: bool,
    pub from_time: Option<DateTime<Utc>>,
    pub to_time: Option<DateTime<Utc>>,
    pub limit: usize,
}

impl Default for HistoryFilter {
    fn default() -> Self {
        Self {
            entry_type: None,
            session_id: None,
            success_only: false,
            from_time: None,
            to_time: None,
            limit: 100,
        }
    }
}

pub struct HistoryManager {
    entries: Arc<RwLock<VecDeque<HistoryEntry>>>,
    history_path: PathBuf,
    max_entries: usize,
}

impl HistoryManager {
    pub fn new() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let history_path = home.join(".wgenty-code").join("history.jsonl");

        Self {
            entries: Arc::new(RwLock::new(VecDeque::new())),
            history_path,
            max_entries: 10000,
        }
    }

    /// Create a HistoryManager with a custom file path (for testing).
    pub fn with_path(history_path: PathBuf) -> Self {
        Self {
            entries: Arc::new(RwLock::new(VecDeque::new())),
            history_path,
            max_entries: 10000,
        }
    }

    pub async fn add(&self, entry: HistoryEntry) -> anyhow::Result<()> {
        // Serialize only the single new entry (append-only) rather than the
        // entire history. This is O(1) per add() instead of O(n).
        let serialized = serde_json::to_string(&entry)?;

        {
            let mut entries = self.entries.write().await;

            if entries.len() >= self.max_entries {
                entries.pop_front();
            }

            entries.push_back(entry);
        } // write lock released

        self.append_line(&serialized).await?;
        Ok(())
    }

    pub async fn get(&self, id: &str) -> Option<HistoryEntry> {
        let entries = self.entries.read().await;
        entries.iter().find(|e| e.id == id).cloned()
    }

    pub async fn list(&self, filter: HistoryFilter) -> Vec<HistoryEntry> {
        let entries = self.entries.read().await;

        let mut result: Vec<HistoryEntry> = entries
            .iter()
            .filter(|e| {
                if let Some(ref entry_type) = filter.entry_type {
                    if e.entry_type != *entry_type {
                        return false;
                    }
                }

                if let Some(ref session_id) = filter.session_id {
                    if e.session_id.as_ref() != Some(session_id) {
                        return false;
                    }
                }

                if filter.success_only && !e.success {
                    return false;
                }

                if let Some(from) = filter.from_time {
                    if e.timestamp < from {
                        return false;
                    }
                }

                if let Some(to) = filter.to_time {
                    if e.timestamp > to {
                        return false;
                    }
                }

                true
            })
            .cloned()
            .collect();

        result.truncate(filter.limit);
        result
    }

    pub async fn search(&self, query: &str) -> Vec<HistoryEntry> {
        let query_lower = query.to_lowercase();
        let entries = self.entries.read().await;

        entries
            .iter()
            .filter(|e| e.content.to_lowercase().contains(&query_lower))
            .cloned()
            .collect()
    }

    pub async fn get_recent(&self, count: usize) -> Vec<HistoryEntry> {
        let entries = self.entries.read().await;
        entries.iter().rev().take(count).cloned().collect()
    }

    pub async fn get_by_type(&self, entry_type: HistoryType, limit: usize) -> Vec<HistoryEntry> {
        let entries = self.entries.read().await;
        entries
            .iter()
            .filter(|e| e.entry_type == entry_type)
            .rev()
            .take(limit)
            .cloned()
            .collect()
    }

    pub async fn clear(&self) -> anyhow::Result<()> {
        let mut entries = self.entries.write().await;
        entries.clear();
        // Truncate the JSONL file on disk.
        if self.history_path.exists() {
            tokio::fs::write(&self.history_path, "").await?;
        }
        Ok(())
    }

    pub async fn stats(&self) -> HistoryStats {
        let entries = self.entries.read().await;

        let mut commands = 0;
        let mut queries = 0;
        let mut tool_calls = 0;
        let mut successful = 0;
        let mut failed = 0;

        for entry in entries.iter() {
            match entry.entry_type {
                HistoryType::Command => commands += 1,
                HistoryType::Query => queries += 1,
                HistoryType::ToolCall => tool_calls += 1,
                _ => {}
            }

            if entry.success {
                successful += 1;
            } else {
                failed += 1;
            }
        }

        HistoryStats {
            total_entries: entries.len(),
            commands,
            queries,
            tool_calls,
            successful,
            failed,
        }
    }

    /// Append a single serialized JSON line to the history file.
    /// This is O(1) per call — the previous implementation rewrote the
    /// entire history file (up to `max_entries` entries) on every `add()`.
    async fn append_line(&self, line: &str) -> anyhow::Result<()> {
        if let Some(parent) = self.history_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        use tokio::io::AsyncWriteExt;
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.history_path)
            .await?;
        file.write_all(line.as_bytes()).await?;
        file.write_all(b"\n").await?;
        Ok(())
    }

    pub async fn load(&self) -> anyhow::Result<()> {
        if !self.history_path.exists() {
            return Ok(());
        }

        let content = tokio::fs::read_to_string(&self.history_path).await?;

        // Support both the new JSONL format (one JSON object per line) and
        // the legacy single-JSON-array format for backward compatibility.
        let loaded: VecDeque<HistoryEntry> = if content.trim_start().starts_with('[') {
            // Legacy format: a single JSON array.
            serde_json::from_str(&content)?
        } else {
            // JSONL format: one JSON object per line.
            content
                .lines()
                .filter(|line| !line.trim().is_empty())
                .map(serde_json::from_str::<HistoryEntry>)
                .collect::<Result<VecDeque<_>, _>>()?
        };

        let mut entries = self.entries.write().await;
        *entries = loaded;

        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryStats {
    pub total_entries: usize,
    pub commands: usize,
    pub queries: usize,
    pub tool_calls: usize,
    pub successful: usize,
    pub failed: usize,
}

impl Default for HistoryManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn add_appends_single_line() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("history.jsonl");
        let mgr = HistoryManager::with_path(path.clone());

        mgr.add(HistoryEntry::new(HistoryType::Command, "cmd-1"))
            .await
            .unwrap();
        mgr.add(HistoryEntry::new(HistoryType::Command, "cmd-2"))
            .await
            .unwrap();

        let content = tokio::fs::read_to_string(&path).await.unwrap();
        let lines: Vec<&str> = content.lines().filter(|l| !l.is_empty()).collect();
        assert_eq!(lines.len(), 2, "each add() should append exactly one line");

        // Each line must be a valid standalone JSON object.
        for line in &lines {
            let _: HistoryEntry = serde_json::from_str(line).unwrap();
        }
    }

    #[tokio::test]
    async fn load_reads_jsonl_format() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("history.jsonl");

        // Write two JSONL lines directly.
        let e1 = HistoryEntry::new(HistoryType::Command, "first");
        let e2 = HistoryEntry::new(HistoryType::Query, "second");
        let content = format!(
            "{}\n{}\n",
            serde_json::to_string(&e1).unwrap(),
            serde_json::to_string(&e2).unwrap()
        );
        tokio::fs::write(&path, &content).await.unwrap();

        let mgr = HistoryManager::with_path(path);
        mgr.load().await.unwrap();

        let recent = mgr.get_recent(10).await;
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].content, "second"); // most recent first
        assert_eq!(recent[1].content, "first");
    }

    #[tokio::test]
    async fn load_reads_legacy_json_array() {
        let tmp = tempfile::tempdir().unwrap();
        // Use the old .json extension to simulate a legacy file.
        let path = tmp.path().join("history.json");

        let e1 = HistoryEntry::new(HistoryType::Command, "legacy-1");
        let e2 = HistoryEntry::new(HistoryType::Query, "legacy-2");
        let legacy = serde_json::to_string_pretty(&vec![e1, e2]).unwrap();
        tokio::fs::write(&path, &legacy).await.unwrap();

        let mgr = HistoryManager::with_path(path);
        mgr.load().await.unwrap();

        let recent = mgr.get_recent(10).await;
        assert_eq!(recent.len(), 2);
    }

    #[tokio::test]
    async fn clear_empties_file_and_memory() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("history.jsonl");
        let mgr = HistoryManager::with_path(path.clone());

        mgr.add(HistoryEntry::new(HistoryType::Command, "cmd"))
            .await
            .unwrap();
        assert!(path.exists());

        mgr.clear().await.unwrap();

        let recent = mgr.get_recent(10).await;
        assert!(recent.is_empty());
        let content = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(content.is_empty());
    }

    #[tokio::test]
    async fn max_entries_eviction() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("history.jsonl");
        let mut mgr = HistoryManager::with_path(path);
        mgr.max_entries = 3;

        for i in 0..5 {
            mgr.add(HistoryEntry::new(HistoryType::Command, &format!("cmd-{i}")))
                .await
                .unwrap();
        }

        let recent = mgr.get_recent(10).await;
        assert_eq!(recent.len(), 3, "should evict oldest beyond max_entries");
        // The oldest two (cmd-0, cmd-1) should have been evicted.
        assert!(recent.iter().all(|e| !e.content.contains("cmd-0")));
        assert!(recent.iter().all(|e| !e.content.contains("cmd-1")));
    }
}
