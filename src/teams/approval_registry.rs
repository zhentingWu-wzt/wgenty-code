//! Process-wide registry of pending approval waiters (s10).
//!
//! `request_approval` tool registers a oneshot sender keyed by `request_id`;
//! `MailboxInbox::drain` resolves it when an `ApprovalResponse` arrives.
//! Per-agent map so multiple subagents don't collide on request ids.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use tokio::sync::oneshot;

type PendingMap = HashMap<String, oneshot::Sender<bool>>;
type AgentMap = HashMap<String, Arc<Mutex<PendingMap>>>;

fn global() -> &'static Mutex<AgentMap> {
    static REG: OnceLock<Mutex<AgentMap>> = OnceLock::new();
    REG.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Get (or create) the pending-approvals handle for `agent_id`.
/// Shared between the subagent's `MailboxInbox` and its `request_approval` tool.
pub fn register_agent(agent_id: &str) -> Arc<Mutex<PendingMap>> {
    let mut map = global().lock().expect("lock poisoned: approval registry");
    map.entry(agent_id.to_string())
        .or_insert_with(|| Arc::new(Mutex::new(HashMap::new())))
        .clone()
}

/// Drop an agent's entry when its subagent loop exits (best-effort cleanup).
pub fn unregister_agent(agent_id: &str) {
    let mut map = global().lock().expect("lock poisoned: approval registry");
    map.remove(agent_id);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_returns_same_handle_for_same_agent() {
        let a = register_agent("agent-X");
        let b = register_agent("agent-X");
        assert!(Arc::ptr_eq(&a, &b));
        unregister_agent("agent-X");
    }

    #[test]
    fn register_returns_distinct_handle_per_agent() {
        let a = register_agent("agent-A");
        let b = register_agent("agent-B");
        assert!(!Arc::ptr_eq(&a, &b));
        unregister_agent("agent-A");
        unregister_agent("agent-B");
    }
}

#[cfg(test)]
mod integration {
    use super::*;
    use crate::teams::mailbox::{Mailbox, TeamMessage};
    use std::path::PathBuf;

    fn tmp_inbox(name: &str) -> (PathBuf, Mailbox) {
        let dir = std::env::temp_dir().join(format!("wgenty-approval-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{}.jsonl", name));
        // clean slate
        let _ = std::fs::remove_file(&path);
        (path.clone(), Mailbox::new(path))
    }

    /// Drive an ApprovalResponse into the pending-approvals map the way
    /// MailboxInbox::drain does, and confirm the waiter unblocks.
    #[tokio::test]
    async fn approval_response_resolves_waiter() {
        let agent = "approver-agent";
        let pending = register_agent(agent);
        let (tx, rx) = oneshot::channel::<bool>();
        pending.lock().unwrap().insert("req-1".into(), tx);

        // Simulate drain delivering an ApprovalResponse.
        let delivered = {
            let mut map = pending.lock().unwrap();
            if let Some(sender) = map.remove("req-1") {
                let _ = sender.send(true);
                true
            } else {
                false
            }
        };
        assert!(delivered);

        let approve = tokio::time::timeout(std::time::Duration::from_secs(1), rx)
            .await
            .expect("no timeout")
            .expect("channel ok");
        assert!(approve);
        unregister_agent(agent);
    }

    #[tokio::test]
    async fn shutdown_request_message_roundtrips() {
        // Verify the serde shape of ShutdownRequest so drain's match arm keys
        // off a stable wire format.
        let (_p, mailbox) = tmp_inbox("shutdown-peer");
        mailbox
            .send(&TeamMessage::ShutdownRequest {
                from: "parent".into(),
                request_id: "s-1".into(),
            })
            .await
            .unwrap();
        let drained = mailbox.receive_all().await.unwrap();
        assert_eq!(drained.len(), 1);
        match &drained[0] {
            TeamMessage::ShutdownRequest { from, request_id } => {
                assert_eq!(from, "parent");
                assert_eq!(request_id, "s-1");
            }
            other => panic!("unexpected message: {other:?}"),
        }
    }
}
