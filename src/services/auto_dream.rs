//! AutoDream Service - Automatic memory consolidation
//!
//! Background memory consolidation that fires the /dream prompt as a forked
//! subagent when time-gate passes AND enough sessions have accumulated.
//!
//! Gate order (cheapest first):
//!   1. Time: hours since lastConsolidatedAt >= minHours
//!   2. Sessions: transcript count with mtime > lastConsolidatedAt >= minSessions
//!   3. Lock: no other process mid-consolidation

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::state::AppState;

/// Default configuration for AutoDream
const DEFAULT_MIN_HOURS: i64 = 24;
const DEFAULT_MIN_SESSIONS: usize = 5;
const SESSION_SCAN_INTERVAL_MS: i64 = 10 * 60 * 1000;

/// AutoDream configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoDreamConfig {
    pub min_hours: i64,
    pub min_sessions: usize,
    pub enabled: bool,
}

impl Default for AutoDreamConfig {
    fn default() -> Self {
        Self {
            min_hours: DEFAULT_MIN_HOURS,
            min_sessions: DEFAULT_MIN_SESSIONS,
            enabled: true,
        }
    }
}

/// Consolidation state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsolidationState {
    pub last_consolidated_at: DateTime<Utc>,
    pub session_count: usize,
    pub is_consolidating: bool,
    pub last_session_scan: DateTime<Utc>,
}

impl Default for ConsolidationState {
    fn default() -> Self {
        Self {
            last_consolidated_at: Utc::now() - Duration::hours(DEFAULT_MIN_HOURS + 1),
            session_count: 0,
            is_consolidating: false,
            last_session_scan: Utc::now(),
        }
    }
}

/// AutoDream service
pub struct AutoDreamService {
    config: AutoDreamConfig,
    consolidation_state: Arc<RwLock<ConsolidationState>>,
    memory_manager: Option<Arc<crate::context::MemoryManager>>,
}

impl AutoDreamService {
    pub fn new(
        _state: Arc<RwLock<AppState>>,
        config: Option<AutoDreamConfig>,
        memory_manager: Option<Arc<crate::context::MemoryManager>>,
    ) -> Self {
        Self {
            config: config.unwrap_or_default(),
            consolidation_state: Arc::new(RwLock::new(ConsolidationState::default())),
            memory_manager,
        }
    }

    pub fn with_config(mut self, config: AutoDreamConfig) -> Self {
        self.config = config;
        self
    }

    pub fn with_memory_manager(mut self, mm: Arc<crate::context::MemoryManager>) -> Self {
        self.memory_manager = Some(mm);
        self
    }

    pub async fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    pub async fn check_and_run(&self) -> anyhow::Result<bool> {
        if !self.config.enabled {
            return Ok(false);
        }

        let mut consolidation = self.consolidation_state.write().await;

        if consolidation.is_consolidating {
            return Ok(false);
        }

        let now = Utc::now();
        let hours_since = (now - consolidation.last_consolidated_at).num_hours();

        if hours_since < self.config.min_hours {
            return Ok(false);
        }

        let scan_interval = chrono::Duration::milliseconds(SESSION_SCAN_INTERVAL_MS);
        if now - consolidation.last_session_scan < scan_interval {
            return Ok(false);
        }

        consolidation.last_session_scan = now;

        let sessions = self
            .count_recent_sessions(&consolidation.last_consolidated_at)
            .await?;

        if sessions < self.config.min_sessions {
            return Ok(false);
        }

        if !self.try_acquire_lock(&mut consolidation).await? {
            return Ok(false);
        }

        drop(consolidation);

        self.run_consolidation().await?;

        let mut consolidation = self.consolidation_state.write().await;
        consolidation.last_consolidated_at = Utc::now();
        consolidation.is_consolidating = false;
        self.save_state(&consolidation).await?;

        Ok(true)
    }

    async fn count_recent_sessions(&self, since: &DateTime<Utc>) -> anyhow::Result<usize> {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let sessions_dir = home.join(".wgenty-code").join("sessions");

        if !sessions_dir.exists() {
            return Ok(0);
        }

        let mut count = 0;
        let entries = std::fs::read_dir(&sessions_dir)?;

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "json") {
                if let Ok(metadata) = entry.metadata() {
                    if let Ok(modified) = metadata.modified() {
                        let modified: DateTime<Utc> = modified.into();
                        if modified > *since {
                            count += 1;
                        }
                    }
                }
            }
        }

        Ok(count)
    }

    async fn try_acquire_lock(
        &self,
        consolidation: &mut ConsolidationState,
    ) -> anyhow::Result<bool> {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let lock_path = home.join(".wgenty-code").join(".consolidation.lock");

        if lock_path.exists() {
            let content = std::fs::read_to_string(&lock_path)?;
            if let Ok(lock_time) = chrono::DateTime::parse_from_rfc3339(&content) {
                let lock_time: DateTime<Utc> = lock_time.with_timezone(&Utc);
                if Utc::now() - lock_time < chrono::Duration::hours(1) {
                    return Ok(false);
                }
            }
        }

        std::fs::write(&lock_path, Utc::now().to_rfc3339())?;
        consolidation.is_consolidating = true;
        Ok(true)
    }

    async fn run_consolidation(&self) -> anyhow::Result<()> {
        tracing::info!("AutoDream: Starting memory consolidation...");

        let Some(ref mm) = self.memory_manager else {
            tracing::warn!("AutoDream: no MemoryManager configured; consolidation skipped");
            return Ok(());
        };

        // Load memories from disk (if not already loaded)
        mm.load().await?;

        // Get count before consolidation
        let status = mm.status().await?;
        let before = status.total_memories;
        if before == 0 {
            tracing::info!("AutoDream: No memories to consolidate");
            return Ok(());
        }

        // Delegate to MemoryManager::consolidate() which uses ConsolidationEngine
        mm.consolidate().await?;

        // Persist consolidated memories
        mm.save().await?;

        let status = mm.status().await?;
        tracing::info!(
            before = before,
            after = status.total_memories,
            "AutoDream: Consolidation complete"
        );

        Ok(())
    }


    async fn save_state(&self, state: &ConsolidationState) -> anyhow::Result<()> {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        let state_path = home.join(".wgenty-code").join(".autodream_state.json");

        let content = serde_json::to_string_pretty(state)?;
        std::fs::write(&state_path, content)?;

        Ok(())
    }

    pub async fn get_status(&self) -> AutoDreamStatus {
        let consolidation = self.consolidation_state.read().await;
        let now = Utc::now();
        let hours_since = (now - consolidation.last_consolidated_at).num_hours();

        AutoDreamStatus {
            enabled: self.config.enabled,
            is_consolidating: consolidation.is_consolidating,
            last_consolidation: consolidation.last_consolidated_at,
            hours_since_last: hours_since,
            sessions_accumulated: consolidation.session_count,
            next_consolidation_in: self.config.min_hours - hours_since,
        }
    }

    pub async fn force_consolidation(&self) -> anyhow::Result<()> {
        let mut consolidation = self.consolidation_state.write().await;
        consolidation.is_consolidating = true;
        drop(consolidation);

        self.run_consolidation().await?;

        let mut consolidation = self.consolidation_state.write().await;
        consolidation.last_consolidated_at = Utc::now();
        consolidation.is_consolidating = false;
        self.save_state(&consolidation).await?;

        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct AutoDreamStatus {
    pub enabled: bool,
    pub is_consolidating: bool,
    pub last_consolidation: DateTime<Utc>,
    pub hours_since_last: i64,
    pub sessions_accumulated: usize,
    pub next_consolidation_in: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_autodream_delegates_to_memory_manager() {
        use crate::context::MemoryManager;
        let mm = std::sync::Arc::new(MemoryManager::new());
        // Add some test memories
        mm.add_memory(
            crate::context::MemoryEntry::new(
                crate::context::MemoryType::Knowledge,
                "test memory",
            )
            .with_importance(0.8),
        )
        .await
        .unwrap();

        let state = std::sync::Arc::new(tokio::sync::RwLock::new(
            crate::state::AppState::default(),
        ));
        let config = AutoDreamConfig {
            min_hours: 0,
            min_sessions: 0,
            enabled: true,
        };
        let service = AutoDreamService::new(state, Some(config), Some(mm.clone()));

        // Force consolidation (bypasses gate)
        let result = service.force_consolidation().await;
        assert!(result.is_ok());
    }
}
