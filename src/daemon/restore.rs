//! Restart-time restore of persisted subagent records into the in-memory
//! agent hierarchy.
//!
//! The agent store is intentionally in-memory (strict local-visibility
//! projections over a HashMap), while terminal subagent state is persisted in
//! the transcript SQLite DB (`subagent_transcripts`, keyed by session). After a
//! daemon restart the store starts empty, so the web tree panel (which polls
//! `GET /agents/directory`) loses every previously-spawned subagent even
//! though the history is still on disk. This module rebuilds those records
//! from the transcript headers and hands them to
//! [`crate::agent::AgentCoordinator::restore_child_records`], making the
//! directory, self-view, and navigation reads see the restored history.
//!
//! The transcript id IS the agent id (both are the child UUID minted by the
//! coordinator at reserve time), so restored nodes are addressable by the same
//! ids the UI already navigates with.

use std::collections::HashMap;

use crate::agent::store::{ChildSummary, RestoredChildRecord};
use crate::tools::context::{AgentId, AgentLifecycleStatus, SessionId};
use crate::transcript::{SubagentTranscriptHeader, SubagentTranscriptStore};

/// Maximum restored children per session. Guards pathological or corrupted
/// databases from flooding the in-memory store on first touch.
const MAX_RESTORED_PER_SESSION: usize = 200;

/// Maps persisted transcript headers for one session into restorable child
/// records rooted at the (fresh) root agent.
///
/// Only terminal rows are restored (`completed` / `failed` / `cancelled`) — a
/// `running` row whose process is gone is dead, not restorable, and must not
/// reappear as a live agent. Rows whose `parent_id` does not resolve to another
/// restored row or to the root are re-parented onto the root (the coordinator's
/// default hierarchy is a single level below root, and `task`-path transcripts
/// are written with a NULL parent). Parent cycles collapse the same way, so a
/// corrupted chain can never build a loop into the tree.
pub(crate) fn restored_records_from_headers(
    root: &AgentId,
    session: &SessionId,
    headers: &[SubagentTranscriptHeader],
) -> Vec<RestoredChildRecord> {
    // Newest-first from list_by_session; process oldest-first so the final
    // vector keeps chronological order.
    let mut headers: Vec<&SubagentTranscriptHeader> = headers.iter().collect();
    headers.reverse();

    let status_of = |h: &SubagentTranscriptHeader| match h.status.as_str() {
        "completed" => Some(AgentLifecycleStatus::Completed),
        "failed" => Some(AgentLifecycleStatus::Failed),
        "cancelled" => Some(AgentLifecycleStatus::Cancelled),
        // pending/running rows are not terminal — dead processes stay dead.
        _ => None,
    };

    // First pass: accept terminal rows as candidate records keyed by their id
    // (= agent id).
    let mut by_agent: HashMap<&str, RestoredChildRecord> = HashMap::new();
    for h in &headers {
        let Some(status) = status_of(h) else {
            continue;
        };
        if by_agent.len() >= MAX_RESTORED_PER_SESSION {
            break;
        }
        let started =
            chrono::DateTime::from_timestamp_millis(h.started_at).unwrap_or_else(chrono::Utc::now);
        let finished = h
            .finished_at
            .and_then(chrono::DateTime::from_timestamp_millis)
            .unwrap_or(started);
        let summary = match (&h.summary, &h.error_message) {
            (Some(text), _) if !text.is_empty() => Some(ChildSummary {
                text: text.clone(),
                error_code: None,
            }),
            (None, Some(msg)) => Some(ChildSummary {
                text: msg.clone(),
                error_code: None,
            }),
            _ => None,
        };
        by_agent.insert(
            h.id.as_str(),
            RestoredChildRecord {
                session_id: session.clone(),
                agent_id: AgentId::new(h.id.clone()),
                // Placeholder parent; resolved in the second pass.
                parent_id: root.clone(),
                depth: 1,
                label: h.label.clone(),
                status,
                summary,
                created_at: started,
                updated_at: finished,
            },
        );
    }

    // Second pass: resolve parent links and depths. A recorded parent is kept
    // only when the chain from it terminates at the root (a NULL parent in a
    // restored row means "child of root"); dangling links and cycles collapse
    // onto the root at depth 1. Depths are memoized so chains resolve once.
    let mut depth_memo: HashMap<&str, usize> = HashMap::new();
    let mut visiting: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let mut records = Vec::with_capacity(by_agent.len());
    for rec in by_agent.into_values() {
        let id = rec.agent_id.as_str();
        let recorded_parent = headers
            .iter()
            .find(|h| h.id == id)
            .and_then(|h| h.parent_id.as_deref());
        let mut rec = rec;
        match recorded_parent {
            // NULL or explicitly-the-root parent: child of root.
            None => {}
            Some(pid) if pid == root.as_str() => {}
            Some(pid) => {
                match chain_depth(pid, &headers, &mut depth_memo, root.as_str(), &mut visiting) {
                    Some(parent_depth) if parent_depth < MAX_RESTORED_PER_SESSION => {
                        rec.parent_id = AgentId::new(pid.to_string());
                        rec.depth = parent_depth + 1;
                    }
                    // Dangling, cyclic, or pathologically deep: root, depth 1.
                    _ => {}
                }
            }
        }
        records.push(rec);
    }
    records
}

/// Depth of `id` within the restored set, where "child of root" is depth 1.
/// Returns None when the chain is dangling, cyclic, or deeper than the restore
/// cap. `visiting` tracks the current walk (cycle guard) while `memo` caches
/// finished results (shared across siblings).
fn chain_depth<'a>(
    id: &'a str,
    headers: &[&'a SubagentTranscriptHeader],
    memo: &mut HashMap<&'a str, usize>,
    root: &str,
    visiting: &mut std::collections::HashSet<&'a str>,
) -> Option<usize> {
    if id == root {
        return Some(0);
    }
    if let Some(&depth) = memo.get(id) {
        return Some(depth);
    }
    if !visiting.insert(id) {
        return None; // cycle
    }
    let header = headers.iter().find(|h| h.id == id)?;
    let depth = match header.parent_id.as_deref() {
        // NULL parent = child of root, which is depth 1.
        None => 1,
        Some(pid) => {
            let parent_depth = chain_depth(pid, headers, memo, root, visiting)?;
            if parent_depth >= MAX_RESTORED_PER_SESSION {
                return None; // pathologically deep — collapse to root
            }
            parent_depth + 1
        }
    };
    visiting.remove(id);
    memo.insert(id, depth);
    Some(depth)
}

/// Restores persisted subagent history for `session_id` into the coordinator.
///
/// Called once per session on first `root_context` after a daemon start
/// (i.e. exactly when the in-memory store has no records for the session yet).
/// Best-effort: transcript-store failures only degrade the tree back to
/// live-only — they never fail the request path that triggered the restore.
pub(crate) async fn restore_session_subagents(
    transcript_store: Option<&std::sync::Arc<SubagentTranscriptStore>>,
    coordinator: &crate::agent::AgentCoordinator,
    subagent_progress: &std::sync::Arc<
        tokio::sync::RwLock<HashMap<String, HashMap<String, crate::progress::SubagentProgress>>>,
    >,
    session_id: &str,
    root: &AgentId,
) {
    let Some(store) = transcript_store else {
        return;
    };
    let headers = match store.list_by_session(session_id) {
        Ok(headers) => headers,
        Err(e) => {
            tracing::warn!(
                target: "wgenty::daemon",
                error = %e,
                session_id,
                "subagent restore: transcript lookup failed; tree stays live-only"
            );
            return;
        }
    };
    if headers.is_empty() {
        return;
    }

    let session = SessionId::new(session_id);
    let records = restored_records_from_headers(root, &session, &headers);
    if records.is_empty() {
        return;
    }

    // Terminal progress entries let the directory/self projections cross-fill
    // tokens, rounds, and the chat-style messages timeline without which the
    // restored tree renders with zeroed metrics.
    let mut progress_entries = Vec::with_capacity(records.len());
    for header in &headers {
        let Some(status) = terminal_progress_status(&header.status) else {
            continue;
        };
        progress_entries.push(crate::progress::SubagentProgress {
            node_id: header.id.clone(),
            parent_id: Some(
                header
                    .parent_id
                    .clone()
                    .unwrap_or_else(|| root.as_str().to_string()),
            ),
            label: header.label.clone(),
            status,
            round: Some(header.actual_rounds as usize),
            max_rounds: None,
            current_tool: None,
            current_params: None,
            action_log: Vec::new(),
            text_snapshot: header.summary.clone(),
            started_at: header.started_at,
            elapsed_ms: header
                .finished_at
                .map(|f| (f - header.started_at).max(0) as u64)
                .unwrap_or(0),
            metadata: None,
            progress_delta: None,
            token_budget_k: None,
            cumulative_tokens: header.total_tokens,
            error_details: None,
            events: Vec::new(),
            messages: Vec::new(),
        });
    }

    if let Err(e) = coordinator.restore_child_records(records).await {
        tracing::warn!(
            target: "wgenty::daemon",
            error = %e,
            session_id,
            "subagent restore: inserting records into agent store failed"
        );
        return;
    }
    {
        let mut store = subagent_progress.write().await;
        let session_map = store.entry(session_id.to_string()).or_default();
        for progress in progress_entries {
            // Never overwrite a live entry written by the current process.
            session_map
                .entry(progress.node_id.clone())
                .or_insert(progress);
        }
    }
    tracing::info!(
        target: "wgenty::daemon",
        session_id,
        "subagent restore: replayed persisted subagent history into agent store"
    );
}

fn terminal_progress_status(db_status: &str) -> Option<crate::progress::SubagentStatus> {
    match db_status {
        "completed" => Some(crate::progress::SubagentStatus::Completed),
        "failed" => Some(crate::progress::SubagentStatus::Failed),
        "cancelled" => Some(crate::progress::SubagentStatus::Cancelled),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(
        id: &str,
        parent: Option<&str>,
        status: &str,
        started: i64,
    ) -> SubagentTranscriptHeader {
        SubagentTranscriptHeader {
            id: id.to_string(),
            session_id: "s".to_string(),
            parent_id: parent.map(str::to_string),
            label: format!("label-{id}"),
            status: status.to_string(),
            started_at: started,
            finished_at: Some(started + 5_000),
            total_tokens: 42,
            actual_rounds: 3,
            error_message: None,
            summary: Some(format!("summary-{id}")),
            root_cause: Default::default(),
            project_path: None,
            node_type: None,
        }
    }

    fn root() -> AgentId {
        AgentId::new("root".to_string())
    }

    #[test]
    fn terminal_headers_become_depth_one_children_of_root() {
        let headers = vec![
            header("a", None, "completed", 1000),
            header("b", None, "failed", 2000),
        ];
        let records = restored_records_from_headers(&root(), &SessionId::new("s"), &headers);
        assert_eq!(records.len(), 2);
        assert!(records.iter().all(|r| r.depth == 1));
        assert!(records.iter().all(|r| r.parent_id.as_str() == "root"));
        let a = records.iter().find(|r| r.agent_id.as_str() == "a").unwrap();
        assert_eq!(a.status, AgentLifecycleStatus::Completed);
        assert_eq!(a.summary.as_ref().unwrap().text, "summary-a");
        let b = records.iter().find(|r| r.agent_id.as_str() == "b").unwrap();
        assert_eq!(b.status, AgentLifecycleStatus::Failed);
    }

    #[test]
    fn non_terminal_headers_are_skipped() {
        let headers = vec![
            header("run", None, "running", 1000),
            header("pend", None, "pending", 1100),
            header("done", None, "completed", 1200),
        ];
        let records = restored_records_from_headers(&root(), &SessionId::new("s"), &headers);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].agent_id.as_str(), "done");
    }

    #[test]
    fn recorded_parent_links_are_kept_when_they_reach_root() {
        // Parent chain: child -> mid -> root. `mid` has no recorded parent
        // (NULL), which resolves to root at depth 1, so child is depth 2.
        let headers = vec![
            header("child", Some("mid"), "completed", 3000),
            header("mid", None, "completed", 1000),
        ];
        let records = restored_records_from_headers(&root(), &SessionId::new("s"), &headers);
        let child = records
            .iter()
            .find(|r| r.agent_id.as_str() == "child")
            .unwrap();
        assert_eq!(child.parent_id.as_str(), "mid");
        assert_eq!(child.depth, 2);
    }

    #[test]
    fn parent_cycles_collapse_onto_root() {
        // a -> b -> a: cycle, neither reaches root.
        let headers = vec![
            header("a", Some("b"), "completed", 1000),
            header("b", Some("a"), "completed", 2000),
        ];
        let records = restored_records_from_headers(&root(), &SessionId::new("s"), &headers);
        assert_eq!(records.len(), 2);
        assert!(records.iter().all(|r| r.parent_id.as_str() == "root"));
        assert!(records.iter().all(|r| r.depth == 1));
    }

    #[test]
    fn dangling_parent_reparents_to_root() {
        let headers = vec![header("orphan", Some("ghost"), "completed", 1000)];
        let records = restored_records_from_headers(&root(), &SessionId::new("s"), &headers);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].parent_id.as_str(), "root");
        assert_eq!(records[0].depth, 1);
    }

    #[test]
    fn restore_is_capped_per_session() {
        let headers: Vec<_> = (0..300)
            .map(|i| header(&format!("n{i}"), None, "completed", i))
            .collect();
        let records = restored_records_from_headers(&root(), &SessionId::new("s"), &headers);
        assert_eq!(records.len(), MAX_RESTORED_PER_SESSION);
    }
}
