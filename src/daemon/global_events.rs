//! Global (daemon-wide, cross-project) event bus: todos changes, background
//! results, permission-mode / model switches, task-group results. Separate
//! envelope and seq space from the per-session `SessionEventHub` so
//! high-frequency session deltas can't starve global events (design §3.1).
//! v1 is live-only — clients realign via the existing GET endpoints.

use serde::{Deserialize, Serialize};

/// One event on the global bus. `seq` is monotonic across the daemon process
/// for client dedup/ordering; it is NOT resumable after a restart.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobalEvent {
    pub seq: u64,
    pub kind: GlobalEventKind,
    /// Kind-specific payload. Cross-project events carry project/session
    /// dimension fields so clients can filter (design §10).
    pub data: serde_json::Value,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GlobalEventKind {
    /// Full todos snapshot (small; YAGNI: no incremental diff).
    TodosChanged,
    BackgroundResult,
    ModeChanged,
    ModelChanged,
    TaskGroupResult,
}

pub type GlobalEventHub = tokio::sync::broadcast::Sender<GlobalEvent>;

/// Hub channel capacity; aligned with the session event hub.
pub const GLOBAL_EVENT_HUB_CAPACITY: usize = 1024;

pub fn new_global_event_hub() -> GlobalEventHub {
    tokio::sync::broadcast::channel(GLOBAL_EVENT_HUB_CAPACITY).0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::state::DaemonState;

    /// Mirrors the construction path used by handlers.rs tests: tempdir-backed
    /// settings so nothing touches the developer's real project state.
    async fn test_daemon_state() -> DaemonState {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut settings = crate::config::Settings::default();
        settings.storage.working_dir = temp.path().to_path_buf();
        // All construction-time I/O finishes inside `DaemonState::new`; the
        // broadcast assertions below don't touch the working dir, so the
        // tempdir can drop when this helper returns.
        DaemonState::new(crate::state::AppState::new(settings)).await
    }

    #[tokio::test]
    async fn broadcast_global_assigns_monotonic_seq_to_all_subscribers() {
        let state = test_daemon_state().await;
        let mut a = state.global_event_hub.subscribe();
        let mut b = state.global_event_hub.subscribe();
        state.broadcast_global(
            GlobalEventKind::ModeChanged,
            serde_json::json!({"mode": "yolo"}),
        );
        state.broadcast_global(
            GlobalEventKind::ModelChanged,
            serde_json::json!({"profile": "p1"}),
        );
        for expected_seq in [1u64, 2] {
            let ea = a.recv().await.expect("subscriber a");
            let eb = b.recv().await.expect("subscriber b");
            assert_eq!(ea.seq, expected_seq);
            assert_eq!(eb.seq, expected_seq);
        }
    }
}
