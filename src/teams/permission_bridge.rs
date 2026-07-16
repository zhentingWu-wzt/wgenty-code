//! Session-scoped bridge for subagent policy `Ask` escalations.
//!
//! Waiters block on oneshot channels with a timeout (fail closed → deny).
//! The root TUI/daemon drains [`PermissionBridge::pending`] and calls
//! [`PermissionBridge::resolve`] after the user chooses Allow once / Always / Deny.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{oneshot, Mutex};

/// Structured approval payload for policy Ask escalations.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StructuredApproval {
    pub request_id: String,
    pub from: String,
    pub kind: String,
    pub tool: String,
    pub policy_reason: String,
    pub session_rule: String,
    #[serde(default)]
    pub paths: Vec<String>,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub risk: Option<String>,
    #[serde(default)]
    pub human_summary: String,
}

impl StructuredApproval {
    pub fn policy_ask(
        request_id: impl Into<String>,
        from: impl Into<String>,
        tool: impl Into<String>,
        policy_reason: impl Into<String>,
        session_rule: impl Into<String>,
    ) -> Self {
        let tool = tool.into();
        let policy_reason = policy_reason.into();
        let human_summary = format!("{tool}: {policy_reason}");
        Self {
            request_id: request_id.into(),
            from: from.into(),
            kind: "policy_ask".to_string(),
            tool,
            policy_reason,
            session_rule: session_rule.into(),
            paths: Vec::new(),
            command: None,
            risk: None,
            human_summary,
        }
    }
}

#[derive(Debug)]
struct PendingEntry {
    approval: StructuredApproval,
    tx: oneshot::Sender<bool>,
}

/// In-memory approval bridge shared by subagents and the root UI.
#[derive(Debug, Default)]
pub struct PermissionBridge {
    inner: Mutex<HashMap<String, PendingEntry>>,
    default_timeout: Duration,
}

impl PermissionBridge {
    pub fn new(default_timeout: Duration) -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            default_timeout,
        }
    }

    pub fn with_timeout_secs(secs: u64) -> Self {
        Self::new(Duration::from_secs(secs.max(1)))
    }

    /// Register a pending approval and wait until resolved or timeout.
    ///
    /// Returns `true` if approved, `false` on deny or timeout (fail closed).
    pub async fn request(&self, approval: StructuredApproval) -> bool {
        self.request_with_timeout(approval, self.default_timeout)
            .await
    }

    pub async fn request_with_timeout(
        &self,
        approval: StructuredApproval,
        timeout: Duration,
    ) -> bool {
        let request_id = approval.request_id.clone();
        let (tx, rx) = oneshot::channel();
        {
            let mut guard = self.inner.lock().await;
            // Replace any stale waiter with the same id.
            guard.insert(request_id.clone(), PendingEntry { approval, tx });
        }

        let result = tokio::time::timeout(timeout, rx).await;
        // Ensure waiter is cleaned up on timeout or drop.
        self.inner.lock().await.remove(&request_id);

        match result {
            Ok(Ok(approved)) => approved,
            Ok(Err(_)) => false, // sender dropped
            Err(_) => false,     // timeout → deny
        }
    }

    /// Snapshot of pending approvals for the root UI.
    pub async fn pending(&self) -> Vec<StructuredApproval> {
        self.inner
            .lock()
            .await
            .values()
            .map(|e| e.approval.clone())
            .collect()
    }

    /// Resolve a pending request.
    ///
    /// Returns `true` if a waiter was found and notified.
    pub async fn resolve(&self, request_id: &str, approved: bool) -> bool {
        let entry = self.inner.lock().await.remove(request_id);
        match entry {
            Some(entry) => entry.tx.send(approved).is_ok(),
            None => false,
        }
    }
}

/// Shared handle type used across root + children.
pub type SharedPermissionBridge = Arc<PermissionBridge>;

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(id: &str) -> StructuredApproval {
        StructuredApproval::policy_ask(
            id,
            "child-a",
            "file_write",
            "outside workspace",
            "path:/tmp/x",
        )
    }

    #[tokio::test]
    async fn approve_resolves_waiter() {
        let bridge = Arc::new(PermissionBridge::new(Duration::from_secs(5)));
        let req = sample("r1");
        let bridge_wait = Arc::clone(&bridge);
        let wait = tokio::spawn(async move { bridge_wait.request(req).await });
        // Wait until the request is registered.
        for _ in 0..50 {
            if bridge.pending().await.iter().any(|p| p.request_id == "r1") {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(
            bridge.pending().await.iter().any(|p| p.request_id == "r1"),
            "request should be pending"
        );
        assert!(bridge.resolve("r1", true).await);
        assert!(wait.await.expect("join"));
        assert!(bridge.pending().await.is_empty());
    }

    #[tokio::test]
    async fn timeout_denies() {
        let bridge = PermissionBridge::new(Duration::from_millis(30));
        let approved = bridge.request(sample("r2")).await;
        assert!(!approved);
        assert!(bridge.pending().await.is_empty());
    }

    #[tokio::test]
    async fn deny_resolves_false() {
        let bridge = Arc::new(PermissionBridge::new(Duration::from_secs(5)));
        let bridge_wait = Arc::clone(&bridge);
        let wait = tokio::spawn(async move { bridge_wait.request(sample("r3")).await });
        for _ in 0..50 {
            if bridge.pending().await.iter().any(|p| p.request_id == "r3") {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(bridge.resolve("r3", false).await);
        assert!(!wait.await.expect("join"));
    }
}
