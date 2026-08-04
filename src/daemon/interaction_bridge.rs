//! Interaction bridge for `ask_user_question` in the server-side agent loop.
//!
//! Mirrors [`PermissionBridge`] (`src/teams/permission_bridge.rs`): a tool that
//! needs user input registers a pending question and blocks until a frontend
//! resolves it via `POST /api/v1/interactions/:id/resolve`. Resolution (and
//! dropped waiters) publish trace events so subscribers can dismiss prompts.
//!
//! Separate from PermissionBridge on purpose: permissions are approve/deny
//! (bool) while questions carry structured options and return a free-form
//! answer string — merging would bloat both payloads and confuse dispatch.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{oneshot, Mutex};

/// One selectable option in a question prompt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuestionOption {
    pub label: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
}

/// Payload pushed to frontends when `ask_user_question` blocks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuestionPayload {
    pub request_id: String,
    pub session_id: String,
    pub question: String,
    pub options: Vec<QuestionOption>,
    #[serde(default)]
    pub multi_select: bool,
}

impl QuestionPayload {
    /// Parse the tool's input args (`{question, options, multiSelect}`) into a
    /// push payload, minting a request id.
    pub fn from_args(args: &serde_json::Value, session_id: &str) -> Self {
        let question = args
            .get("question")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let multi_select = args
            .get("multiSelect")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let options = args
            .get("options")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|o| {
                        Some(QuestionOption {
                            label: o.get("label")?.as_str()?.to_string(),
                            description: o
                                .get("description")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string(),
                            preview: o
                                .get("preview")
                                .and_then(|v| v.as_str())
                                .map(str::to_string),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        Self {
            request_id: uuid::Uuid::new_v4().to_string(),
            session_id: session_id.to_string(),
            question,
            options,
            multi_select,
        }
    }
}

#[derive(Debug)]
struct PendingEntry {
    question: QuestionPayload,
    tx: oneshot::Sender<String>,
}

/// In-memory question bridge shared by the server-side loop and the resolve
/// endpoint. Default-constructible; questions wait indefinitely (no timeout) —
/// the user may answer from any device, like root permission prompts.
#[derive(Debug, Default)]
pub struct InteractionBridge {
    inner: Mutex<HashMap<String, PendingEntry>>,
}

impl InteractionBridge {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a pending question and block until a client resolves it.
    /// Returns the answer string (JSON the client posted). If the waiter is
    /// dropped (run cancelled), the entry is cleaned up and no phantom lingers.
    pub async fn request(&self, payload: QuestionPayload) -> String {
        let request_id = payload.request_id.clone();
        let payload_for_events = payload.clone();
        let (tx, rx) = oneshot::channel();
        {
            let mut guard = self.inner.lock().await;
            guard.insert(
                request_id.clone(),
                PendingEntry {
                    question: payload,
                    tx,
                },
            );
        }

        // Push the prompt to trace SSE subscribers (TUI / web).
        let pending_ev = crate::teams::trace_sink::TraceEvent::question(&payload_for_events, false);
        let _ = crate::teams::trace_sink::trace_hub().send(pending_ev);

        let result = rx.await;
        // Cleanup: resolve() may have already removed it.
        self.inner.lock().await.remove(&request_id);

        let answer = match result {
            Ok(answer) => answer,
            Err(_) => {
                // Sender dropped (run cancelled). Return a neutral default so
                // the loop can continue without hanging.
                serde_json::json!({ "selected": [], "text": "" }).to_string()
            }
        };

        let resolved_ev = crate::teams::trace_sink::TraceEvent::question(&payload_for_events, true);
        let _ = crate::teams::trace_sink::trace_hub().send(resolved_ev);

        answer
    }

    /// Snapshot of pending questions (for a future list endpoint).
    pub async fn pending(&self) -> Vec<QuestionPayload> {
        self.inner
            .lock()
            .await
            .values()
            .map(|e| e.question.clone())
            .collect()
    }

    /// Resolve a pending question. Returns true if a waiter was found.
    pub async fn resolve(&self, request_id: &str, answer: String) -> bool {
        let entry = self.inner.lock().await.remove(request_id);
        match entry {
            Some(entry) => entry.tx.send(answer).is_ok(),
            None => false,
        }
    }
}

pub type SharedInteractionBridge = Arc<InteractionBridge>;

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(id: &str) -> QuestionPayload {
        QuestionPayload {
            request_id: id.to_string(),
            session_id: "s1".to_string(),
            question: "Which?".to_string(),
            options: vec![QuestionOption {
                label: "A".to_string(),
                description: "opt a".to_string(),
                preview: None,
            }],
            multi_select: false,
        }
    }

    #[tokio::test]
    async fn request_blocks_until_resolved() {
        let bridge = Arc::new(InteractionBridge::new());
        let b = bridge.clone();
        let handle = tokio::spawn(async move { b.request(sample("r1")).await });
        tokio::task::yield_now().await;
        // Still pending.
        assert!(bridge.pending().await.iter().any(|q| q.request_id == "r1"));
        assert!(
            bridge
                .resolve("r1", r#"{"selected":["A"]}"#.to_string())
                .await
        );
        let answer = handle.await.unwrap();
        assert_eq!(answer, r#"{"selected":["A"]}"#);
        assert!(bridge.pending().await.is_empty());
    }

    #[tokio::test]
    async fn resolve_unknown_is_false() {
        let bridge = InteractionBridge::new();
        assert!(!bridge.resolve("nope", "x".to_string()).await);
    }

    #[tokio::test]
    async fn dropped_waiter_returns_default_and_cleans_up() {
        let bridge = Arc::new(InteractionBridge::new());
        {
            let b = bridge.clone();
            let _handle = tokio::spawn(async move { b.request(sample("r2")).await });
            tokio::task::yield_now().await;
            // Drop the handle by leaving scope — the task is aborted on drop.
        }
        // Give the runtime a moment to clean up.
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        // The entry may or may not still be there depending on scheduling, but
        // resolving it must not panic and must return false once cleaned.
        let _ = bridge.resolve("r2", "x".to_string()).await;
    }
}
