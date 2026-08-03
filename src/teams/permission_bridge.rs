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

    /// Register a pending approval (fail closed on timeout, deny, or drop).
    ///
    /// Returns `true` if approved, `false` on deny or timeout (fail closed).
    pub async fn request_with_timeout(
        &self,
        approval: StructuredApproval,
        timeout: Duration,
    ) -> bool {
        let (request_id, approval_for_events, rx) = self.enqueue(approval).await;
        let result = tokio::time::timeout(timeout, rx).await;
        let approved = match result {
            Ok(Ok(approved)) => approved,
            Ok(Err(_)) => false, // sender dropped
            Err(_) => false,     // timeout → deny
        };
        self.finish(&request_id, &approval_for_events, approved)
            .await
    }

    /// Register a pending approval and wait with no deadline.
    ///
    /// Used by root server loops where the human decides when to answer.
    /// Returns `false` if the sender is dropped without resolving (fail closed).
    pub async fn request_indefinite(&self, approval: StructuredApproval) -> bool {
        let (request_id, approval_for_events, rx) = self.enqueue(approval).await;
        let approved = rx.await.unwrap_or(false); // sender dropped → deny
        self.finish(&request_id, &approval_for_events, approved)
            .await
    }

    /// Shared body: insert the waiter into `inner` and publish the pending
    /// trace event. Returns the request id, an approval copy for the resolved
    /// trace, and the oneshot receiver.
    ///
    /// The approval copy is needed because `resolve()` removes the entry
    /// itself, so the waiter can't read the approval back after wake-up.
    async fn enqueue(
        &self,
        approval: StructuredApproval,
    ) -> (String, StructuredApproval, oneshot::Receiver<bool>) {
        let request_id = approval.request_id.clone();
        let approval_for_events = approval.clone();
        let (tx, rx) = oneshot::channel();
        {
            let mut guard = self.inner.lock().await;
            // Replace any stale waiter with the same id.
            guard.insert(request_id.clone(), PendingEntry { approval, tx });
        }

        // Notify SSE subscribers (TUI / web) that a permission prompt is ready,
        // so they don't have to poll /tools/pending-permissions. Use `from` as
        // the session hint; the live stream is global anyway.
        let pending_ev = crate::teams::trace_sink::TraceEvent::permission(
            &approval_for_events,
            &approval_for_events.from,
            false,
        );
        let _ = crate::teams::trace_sink::trace_hub().send(pending_ev);

        (request_id, approval_for_events, rx)
    }

    /// Shared cleanup: remove the entry and publish the resolved trace event
    /// (approved / denied / timed out) so the UI can dismiss a prompt answered
    /// elsewhere or expired. `resolve()` may have already removed the entry —
    /// remove is a no-op on a missing key.
    async fn finish(
        &self,
        request_id: &str,
        approval: &StructuredApproval,
        approved: bool,
    ) -> bool {
        self.inner.lock().await.remove(request_id);

        let resolved_ev =
            crate::teams::trace_sink::TraceEvent::permission(approval, &approval.from, true);
        let _ = crate::teams::trace_sink::trace_hub().send(resolved_ev);

        approved
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
    /// Removes the entry and signals its oneshot. The resolved trace event is
    /// published by the waiter in `finish` (which holds an approval copy for
    /// exactly this purpose), so resolution emits exactly one event regardless
    /// of path (explicit resolve, timeout, or drop).
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
    async fn indefinite_waits_until_resolved() {
        let bridge = Arc::new(PermissionBridge::with_timeout_secs(1)); // timeout 与此路径无关
        let approval = StructuredApproval::policy_ask(
            "r1".to_string(),
            "sess".to_string(),
            "file_edit".to_string(),
            "reason".to_string(),
            "path:/x".to_string(),
        );
        let other = Arc::clone(&bridge);
        let waiter = tokio::spawn(async move { other.request_indefinite(approval).await });
        // 等 pending 注册后 resolve；不应在 1s 默认超时时返回
        tokio::time::sleep(Duration::from_millis(1200)).await;
        assert!(!waiter.is_finished()); // 超过了 default_timeout 仍在挂起
        let pending = bridge.pending().await;
        assert!(bridge.resolve(&pending[0].request_id, true).await);
        assert!(waiter.await.expect("join"));
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
