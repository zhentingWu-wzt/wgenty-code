//! History Management - Command and query history

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
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
    /// Maximum file size in bytes before rotation is triggered (default: 10 MiB).
    pub max_file_size: usize,
    /// Maximum number of rotation files to keep (default: 5).
    pub max_rotation_files: usize,
}

impl HistoryManager {
    pub fn new() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let history_path = home.join(".wgenty-code").join("history.jsonl");

        Self {
            entries: Arc::new(RwLock::new(VecDeque::new())),
            history_path,
            max_entries: 10000,
            max_file_size: 10 * 1024 * 1024, // 10 MiB
            max_rotation_files: 5,
        }
    }

    /// Create a HistoryManager with a custom file path (for testing).
    pub fn with_path(history_path: PathBuf) -> Self {
        Self {
            entries: Arc::new(RwLock::new(VecDeque::new())),
            history_path,
            max_entries: 10000,
            max_file_size: 10 * 1024 * 1024, // 10 MiB
            max_rotation_files: 5,
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

        // Check if the file has grown past the rotation threshold.
        if let Ok(metadata) = tokio::fs::metadata(&self.history_path).await {
            if metadata.len() > self.max_file_size as u64 {
                self.rotate().await?;
            }
        }

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

    /// Rotate the current history file: compress it to a timestamped
    /// `.jsonl.gz` file, truncate the current file, and clean up excess
    /// rotation files.
    async fn rotate(&self) -> anyhow::Result<()> {
        // Only rotate if there is content to rotate.
        let content = tokio::fs::read(&self.history_path).await?;
        if content.is_empty() {
            return Ok(());
        }

        // Determine rotation file path.
        let timestamp = Utc::now().timestamp();
        let parent = self.history_path.parent().unwrap_or_else(|| Path::new("."));
        let rot_filename = format!("history.{}.jsonl.gz", timestamp);
        let rot_path = parent.join(&rot_filename);

        // Gzip-compress and write the rotation file.
        let rot_file = std::fs::File::create(&rot_path)?;
        let mut encoder = flate2::write::GzEncoder::new(rot_file, flate2::Compression::default());
        encoder.write_all(&content)?;
        encoder.finish()?;

        // Truncate the current JSONL file.
        tokio::fs::write(&self.history_path, "").await?;

        // Remove rotation files beyond the retention limit.
        self.cleanup_old_rotations().await?;

        Ok(())
    }

    /// Remove the oldest rotation files when the count exceeds
    /// `max_rotation_files`.
    async fn cleanup_old_rotations(&self) -> anyhow::Result<()> {
        let parent = match self.history_path.parent() {
            Some(p) => p,
            None => return Ok(()),
        };

        let mut rot_files: Vec<PathBuf> = Vec::new();
        let mut dir = tokio::fs::read_dir(parent).await?;
        while let Some(entry) = dir.next_entry().await? {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with("history.") && name_str.ends_with(".jsonl.gz") {
                rot_files.push(entry.path());
            }
        }

        // Sort descending by filename (newer timestamps sort higher).
        rot_files.sort_by(|a, b| b.file_name().cmp(&a.file_name()));

        // Remove files beyond the retention limit.
        if rot_files.len() > self.max_rotation_files {
            for path in &rot_files[self.max_rotation_files..] {
                let _ = tokio::fs::remove_file(path).await;
            }
        }

        Ok(())
    }

    /// Decompress and parse a single `.jsonl.gz` rotation file.
    async fn load_gz_file(&self, path: &Path) -> anyhow::Result<Vec<HistoryEntry>> {
        let compressed = tokio::fs::read(path).await?;
        let mut decoder = flate2::read::GzDecoder::new(&compressed[..]);
        let mut decompressed = String::new();
        decoder.read_to_string(&mut decompressed)?;

        let entries: Vec<HistoryEntry> = decompressed
            .lines()
            .filter(|l| !l.trim().is_empty())
            .filter_map(|line| serde_json::from_str::<HistoryEntry>(line).ok())
            .collect();

        Ok(entries)
    }

    pub async fn load(&self) -> anyhow::Result<()> {
        let mut all_entries: Vec<HistoryEntry> = Vec::new();

        // 1. Load from rotation (.jsonl.gz) files in the same directory.
        if let Some(parent) = self.history_path.parent() {
            let mut dir = tokio::fs::read_dir(parent).await?;
            while let Some(entry) = dir.next_entry().await? {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                if name_str.starts_with("history.") && name_str.ends_with(".jsonl.gz") {
                    if let Ok(entries) = self.load_gz_file(&entry.path()).await {
                        all_entries.extend(entries);
                    }
                    // Corrupt / unreadable gz files are silently skipped.
                }
            }
        }

        // 2. Load the current JSONL file.
        if self.history_path.exists() {
            let content = tokio::fs::read_to_string(&self.history_path).await?;

            if content.trim_start().starts_with('[') {
                // Legacy format: a single JSON array.
                let legacy: Vec<HistoryEntry> = serde_json::from_str(&content)?;
                all_entries.extend(legacy);
            } else {
                // JSONL format: one JSON object per line.
                for line in content.lines() {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    if let Ok(entry) = serde_json::from_str::<HistoryEntry>(trimmed) {
                        all_entries.push(entry);
                    }
                    // Skip lines that fail to parse (corrupt entries).
                }
            }
        }

        // 3. Sort by timestamp to preserve chronological order.
        all_entries.sort_by_key(|e| e.timestamp);

        let mut entries = self.entries.write().await;
        *entries = VecDeque::from(all_entries);

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

    // --- Rotation Tests ---

    /// When the history file exceeds max_file_size, add() should trigger
    /// rotation: compress current file, create a new empty one.
    #[tokio::test]
    async fn rotate_triggers_on_file_size_threshold() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("history.jsonl");
        let mut mgr = HistoryManager::with_path(path.clone());
        mgr.max_file_size = 100; // tiny threshold to trigger quickly
        mgr.max_rotation_files = 3;

        // Add entries until the file exceeds the threshold.
        for i in 0..30 {
            mgr.add(HistoryEntry::new(
                HistoryType::Command,
                &format!("cmd-{:04}", i),
            ))
            .await
            .unwrap();
        }

        // A rotation (.jsonl.gz) file should have been created.
        let rotation_files: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".jsonl.gz"))
            .collect();
        assert!(
            !rotation_files.is_empty(),
            "rotation should create at least one .jsonl.gz file"
        );

        // The current jsonl file should still exist but be smaller.
        assert!(path.exists(), "current history.jsonl should still exist");
    }

    /// load() should merge entries from all rotation files + the current
    /// JSONL file, preserving chronological order.
    #[tokio::test]
    async fn load_merges_entries_from_rotated_files() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("history.jsonl");

        // Simulate a rotated file from the past.
        let e_old = HistoryEntry::new(HistoryType::Command, "rotated-cmd");
        let json_old = serde_json::to_string(&e_old).unwrap();

        let rot_path = tmp.path().join("history.1710000000.jsonl.gz");
        {
            let file = std::fs::File::create(&rot_path).unwrap();
            let mut encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
            use std::io::Write;
            writeln!(encoder, "{}", json_old).unwrap();
            encoder.finish().unwrap();
        }

        // Write the current JSONL file.
        let e_new = HistoryEntry::new(HistoryType::Query, "current-query");
        let content = serde_json::to_string(&e_new).unwrap() + "\n";
        tokio::fs::write(&path, &content).await.unwrap();

        let mgr = HistoryManager::with_path(path);
        mgr.load().await.unwrap();

        let all = mgr.get_recent(100).await;
        assert_eq!(
            all.len(),
            2,
            "should load entries from both rotated and current files"
        );
        let contents: Vec<&str> = all.iter().map(|e| e.content.as_str()).collect();
        assert!(contents.contains(&"rotated-cmd"));
        assert!(contents.contains(&"current-query"));
    }

    /// When rotation files exceed max_rotation_files, the oldest ones
    /// should be cleaned up.
    #[tokio::test]
    async fn rotation_cleans_up_excess_files() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("history.jsonl");

        // Pre-create several old rotation files.
        for ts in [1000u64, 2000, 3000, 4000, 5000, 6000, 7000] {
            let rot_path = tmp.path().join(format!("history.{}.jsonl.gz", ts));
            let file = std::fs::File::create(&rot_path).unwrap();
            let mut encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
            use std::io::Write;
            let e = HistoryEntry::new(HistoryType::Command, &format!("cmd-{}", ts));
            writeln!(encoder, "{}", serde_json::to_string(&e).unwrap()).unwrap();
            encoder.finish().unwrap();
        }

        let mut mgr = HistoryManager::with_path(path.clone());
        mgr.max_file_size = 100; // trigger rotation quickly
        mgr.max_rotation_files = 3; // only keep 3

        // Trigger rotation via add().
        for i in 0..30 {
            mgr.add(HistoryEntry::new(
                HistoryType::Command,
                &format!("x{:04}", i),
            ))
            .await
            .unwrap();
        }

        // Count remaining rotation files.
        let rot_count = std::fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".jsonl.gz"))
            .count();

        assert!(
            rot_count <= 3,
            "should keep at most max_rotation_files (3), got {}",
            rot_count
        );
    }

    /// load() should handle corrupt gz files gracefully by skipping them.
    #[tokio::test]
    async fn load_skips_corrupt_rotation_files() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("history.jsonl");

        // Create a corrupt .gz file (just random bytes).
        let bad_rot = tmp.path().join("history.1710000001.jsonl.gz");
        tokio::fs::write(&bad_rot, b"this is not valid gzip")
            .await
            .unwrap();

        // Write a valid current file.
        let e = HistoryEntry::new(HistoryType::Command, "good-one");
        tokio::fs::write(&path, serde_json::to_string(&e).unwrap() + "\n")
            .await
            .unwrap();

        let mgr = HistoryManager::with_path(path);
        // Should not panic / error out — corrupted files are skipped.
        mgr.load().await.unwrap();

        let all = mgr.get_recent(100).await;
        assert_eq!(all.len(), 1, "should load only the valid entry");
        assert_eq!(all[0].content, "good-one");
    }
}
