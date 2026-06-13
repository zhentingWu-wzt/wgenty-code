//! Storage - Persistent storage backend

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

use super::MemoryEntry;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StorageBackend {
    File,
    Memory,
}

pub struct Storage {
    backend: StorageBackend,
    path: PathBuf,
    cache: Arc<RwLock<Vec<MemoryEntry>>>,
}

/// Validate that a memory ID is safe for use in filesystem paths.
/// Rejects empty IDs, path separators, null bytes, and Windows reserved names.
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
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    if RESERVED.contains(&stem) {
        return false;
    }
    true
}

impl Storage {
    pub fn new(path: PathBuf) -> Self {
        Self {
            backend: StorageBackend::File,
            path,
            cache: Arc::new(RwLock::new(Vec::new())),
        }
    }

    pub fn with_backend(mut self, backend: StorageBackend) -> Self {
        self.backend = backend;
        self
    }

    pub async fn save_memory(&self, entry: &MemoryEntry) -> anyhow::Result<()> {
        match self.backend {
            StorageBackend::File => {
                if !validate_id(&entry.id) {
                    anyhow::bail!("Invalid memory ID for filesystem path: {:?}", entry.id);
                }
                let file_path = self.path.join(format!("{}.json", entry.id));
                let content = serde_json::to_string_pretty(entry)?;

                // Write to a temp file then atomically rename to avoid data
                // loss if the process crashes mid-write.
                let tmp_path = file_path.with_extension("json.tmp");
                tokio::fs::write(&tmp_path, &content).await?;
                tokio::fs::rename(&tmp_path, &file_path).await?;
            }
            StorageBackend::Memory => {
                let mut cache = self.cache.write().await;
                // Replace existing entry with same ID, or push new
                if let Some(existing) = cache.iter_mut().find(|e| e.id == entry.id) {
                    *existing = entry.clone();
                } else {
                    cache.push(entry.clone());
                }
            }
        }

        Ok(())
    }

    pub async fn load_memory(&self, id: &str) -> anyhow::Result<Option<MemoryEntry>> {
        match self.backend {
            StorageBackend::File => {
                if !validate_id(id) {
                    return Ok(None);
                }
                let file_path = self.path.join(format!("{}.json", id));

                // Skip the exists() TOCTOU check — read_to_string returns
                // a clear NotFound error if the file doesn't exist.
                match tokio::fs::read_to_string(&file_path).await {
                    Ok(content) => {
                        let entry: MemoryEntry = serde_json::from_str(&content)?;
                        Ok(Some(entry))
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                    Err(e) => Err(e.into()),
                }
            }
            StorageBackend::Memory => {
                let cache = self.cache.read().await;
                Ok(cache.iter().find(|e| e.id == id).cloned())
            }
        }
    }

    pub async fn delete_memory(&self, id: &str) -> anyhow::Result<()> {
        match self.backend {
            StorageBackend::File => {
                if !validate_id(id) {
                    return Ok(());
                }
                let file_path = self.path.join(format!("{}.json", id));
                match tokio::fs::remove_file(&file_path).await {
                    Ok(()) => Ok(()),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                    Err(e) => Err(e.into()),
                }
            }
            StorageBackend::Memory => {
                let mut cache = self.cache.write().await;
                cache.retain(|e| e.id != id);
                Ok(())
            }
        }
    }

    pub async fn load_all(&self) -> anyhow::Result<Vec<MemoryEntry>> {
        match self.backend {
            StorageBackend::File => {
                if !self.path.exists() {
                    return Ok(Vec::new());
                }

                let mut entries = Vec::new();
                let mut dir = tokio::fs::read_dir(&self.path).await?;

                while let Some(entry) = dir.next_entry().await? {
                    let path = entry.path();
                    if path.extension().map(|e| e == "json").unwrap_or(false) {
                        if let Ok(content) = tokio::fs::read_to_string(&path).await {
                            if let Ok(memory) = serde_json::from_str::<MemoryEntry>(&content) {
                                entries.push(memory);
                            }
                        }
                    }
                }

                Ok(entries)
            }
            StorageBackend::Memory => {
                let cache = self.cache.read().await;
                Ok(cache.clone())
            }
        }
    }

    pub async fn save_all(&self, entries: &[MemoryEntry]) -> anyhow::Result<()> {
        tokio::fs::create_dir_all(&self.path).await?;

        for entry in entries {
            self.save_memory(entry).await?;
        }

        Ok(())
    }

    pub async fn clear(&self) -> anyhow::Result<()> {
        match self.backend {
            StorageBackend::File => {
                if self.path.exists() {
                    // Use a temp directory + rename for atomicity
                    let tmp_path = self.path.with_extension("memory.clearing");
                    tokio::fs::rename(&self.path, &tmp_path).await?;
                    tokio::fs::create_dir_all(&self.path).await?;
                    // Best-effort cleanup of old directory
                    let _ = tokio::fs::remove_dir_all(&tmp_path).await;
                }
            }
            StorageBackend::Memory => {
                let mut cache = self.cache.write().await;
                cache.clear();
            }
        }

        Ok(())
    }

    pub async fn size(&self) -> anyhow::Result<u64> {
        match self.backend {
            StorageBackend::File => {
                if !self.path.exists() {
                    return Ok(0);
                }

                let mut total_size = 0u64;
                let mut dir = tokio::fs::read_dir(&self.path).await?;

                while let Some(entry) = dir.next_entry().await? {
                    let metadata = entry.metadata().await?;
                    total_size += metadata.len();
                }

                Ok(total_size)
            }
            StorageBackend::Memory => {
                let cache = self.cache.read().await;
                // Approximate the heap size by summing serialized sizes
                // (size_of_val on Vec only returns the header, not heap data)
                let approx: u64 = cache
                    .iter()
                    .map(|e| {
                        serde_json::to_string(e)
                            .map(|s| s.len() as u64)
                            .unwrap_or(0)
                    })
                    .sum();
                Ok(approx)
            }
        }
    }

    pub async fn count(&self) -> anyhow::Result<usize> {
        match self.backend {
            StorageBackend::File => {
                if !self.path.exists() {
                    return Ok(0);
                }

                let mut count = 0;
                let mut dir = tokio::fs::read_dir(&self.path).await?;

                while let Some(entry) = dir.next_entry().await? {
                    if entry
                        .path()
                        .extension()
                        .map(|e| e == "json")
                        .unwrap_or(false)
                    {
                        count += 1;
                    }
                }

                Ok(count)
            }
            StorageBackend::Memory => {
                let cache = self.cache.read().await;
                Ok(cache.len())
            }
        }
    }
}

impl Default for Storage {
    fn default() -> Self {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        Self::new(home.join(".wgenty-code").join("memory"))
    }
}
