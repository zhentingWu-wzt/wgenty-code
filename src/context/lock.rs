//! Cross-process advisory lock for memory consolidation.
//!
//! Extracted from `mod.rs` to keep the lock mechanics isolated from the
//! `MemoryManager` facade. The lock is `pub(super)` so only the `context`
//! module (`MemoryManager::consolidate`) can use it.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::Context as _;

use crate::context::storage::Storage;

/// Cross-process advisory lock for memory consolidation.
///
/// `MemoryManager::consolidate()` holds an in-process `RwLock`, but that does
/// not protect against two separate `wgenty-code memory dream` processes
/// running concurrently against the same `~/.wgenty-code/memory` directory.
/// This lock uses a lock-file with a PID + timestamp to serialize
/// consolidation across processes.
///
/// Stale locks (older than `STALE_AFTER` or whose PID is no longer alive) are
/// reclaimed so a crashed process does not permanently block consolidation.
pub(super) struct ConsolidationFileLock {
    lock_path: PathBuf,
}

/// A lock is considered stale after this duration and can be reclaimed.
const LOCK_STALE_AFTER_SECS: i64 = 30 * 60;

impl ConsolidationFileLock {
    pub(super) async fn acquire(storage: &Storage) -> anyhow::Result<Self> {
        use tokio::io::AsyncWriteExt;

        let lock_path = storage.path().join(".consolidation.lock");

        // Ensure the directory exists.
        if let Some(parent) = lock_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        loop {
            // Atomically create the lock file with create_new(true) so that
            // only one process can hold it at a time.
            let create_result = tokio::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&lock_path)
                .await;

            match create_result {
                Ok(mut file) => {
                    // We created the file — write our PID + timestamp.
                    let pid = std::process::id();
                    let ts = chrono::Utc::now().to_rfc3339();
                    let content = format!("{}\n{}\n", pid, ts);
                    file.write_all(content.as_bytes())
                        .await
                        .context("failed to write consolidation lock file")?;
                    drop(file);
                    return Ok(Self { lock_path });
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    // Lock exists — check if it's stale.
                    if Self::is_stale(&lock_path).await {
                        tracing::warn!("consolidation lock is stale; reclaiming");
                        // Best-effort removal; race is acceptable (worst case
                        // both processes remove then one wins create_new).
                        let _ = tokio::fs::remove_file(&lock_path).await;
                        continue;
                    }
                    // Wait and retry.
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                }
                Err(e) => {
                    return Err(e).context("failed to create consolidation lock file");
                }
            }
        }
    }

    async fn is_stale(lock_path: &std::path::Path) -> bool {
        let content = match tokio::fs::read_to_string(lock_path).await {
            Ok(c) => c,
            Err(_) => return false,
        };
        let mut lines = content.lines();
        let pid_str = lines.next().and_then(|s| s.trim().parse::<u32>().ok());
        let ts_str = lines.next().map(|s| s.trim());

        // If we can read the PID, check liveness portably without pulling in
        // a `libc` dependency: spawn the platform-native `kill -0` (Unix) or
        // `tasklist` filter (Windows). If the check itself fails (e.g. the
        // helper binary is missing), fall through to the timestamp guard so
        // we never block consolidation forever.
        if let Some(pid) = pid_str {
            if Self::pid_alive(pid) {
                return false;
            }
        }

        // PID is dead or unparseable — check timestamp as a secondary guard.
        if let Some(ts) = ts_str {
            if let Ok(lock_time) = chrono::DateTime::parse_from_rfc3339(ts) {
                let lock_time: chrono::DateTime<chrono::Utc> =
                    lock_time.with_timezone(&chrono::Utc);
                let age = (chrono::Utc::now() - lock_time).num_seconds();
                return age > LOCK_STALE_AFTER_SECS;
            }
        }

        // Can't parse anything — treat as stale so we don't block forever.
        true
    }

    /// Check whether a process is alive, portably, without a `libc` dependency.
    ///
    /// Uses the platform-native helper (`kill -0` on Unix, `tasklist` on
    /// Windows). If the helper is unavailable or errors, returns `false` so
    /// the caller falls back to the timestamp-based staleness guard.
    fn pid_alive(pid: u32) -> bool {
        #[cfg(unix)]
        {
            std::process::Command::new("kill")
                .arg("-0")
                .arg(pid.to_string())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
        }
        #[cfg(not(unix))]
        {
            // On Windows, `tasklist /FI "PID eq <pid>"` lists the process if
            // it is running. This is heavier than Unix `kill -0` but avoids
            // a Win32 API dependency.
            std::process::Command::new("tasklist")
                .args(["/FI", &format!("PID eq {}", pid), "/NH"])
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).contains(&pid.to_string()))
                .unwrap_or(false)
        }
    }
}

impl Drop for ConsolidationFileLock {
    fn drop(&mut self) {
        // Best-effort lock removal on drop. Synchronous removal is fine here
        // because this runs at the end of `consolidate()` and must not be
        // skipped even if the async runtime is shutting down.
        let _ = std::fs::remove_file(&self.lock_path);
    }
}

/// RAII guard that resets the `consolidating` flag on drop, ensuring
/// it is always cleared even when `consolidate()` returns early via `?`.
pub(super) struct ConsolidatingGuard {
    pub(super) flag: Arc<AtomicBool>,
}

impl Drop for ConsolidatingGuard {
    fn drop(&mut self) {
        self.flag.store(false, Ordering::SeqCst);
    }
}
