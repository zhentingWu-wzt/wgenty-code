//! Trace sink: async buffered JSONL writer for subagent progress events.
//!
//! Driven by the existing `ProgressCallback` (`Arc<dyn Fn(SubagentProgress)>`).
//! Each progress update is converted to a compact `TraceEvent` (with sensitive
//! params redacted via `failure_diagnostics::redact_params`) and handed to a
//! bounded mpsc channel. An independent writer task drains the channel in
//! batches and appends one JSON object per line to
//! `<trace_dir>/<session_id>.jsonl` (file mode 0600, dir 0700 on unix).
//!
//! The `ProgressCallback` closure only performs a non-blocking `try_send`, so
//! the agent loop is never blocked by disk I/O. See design D3.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::agent::progress::{ProgressCallback, SubagentProgress};
use crate::teams::failure_diagnostics::redact_params;

/// Bounded channel capacity for the file writer. The writer batches flushes,
/// so a modest buffer absorbs bursts without blocking the agent loop.
const TRACE_CHANNEL_CAPACITY: usize = 1024;

/// A single JSONL trace event persisted to `<trace_dir>/<session_id>.jsonl`.
///
/// Compact and explicitly typed so external tooling (tail / Perfetto import)
/// and the future daemon SSE endpoint (3.3/3.4) share a stable schema. All
/// tool-param fields are redacted before serialization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceEvent {
    /// Unix epoch milliseconds (wall clock) at emit time.
    pub ts: i64,
    pub session_id: String,
    pub node_id: String,
    pub parent_id: Option<String>,
    pub label: String,
    /// `SubagentStatus` variant name (serde-serialized).
    pub status: String,
    pub round: Option<usize>,
    pub current_tool: Option<String>,
    /// Redacted tool params: parsed JSON (redacted) when `current_params` was a
    /// JSON string, otherwise the raw summary string.
    pub current_params: Option<serde_json::Value>,
    pub elapsed_ms: u64,
    pub progress_delta: Option<f32>,
    pub token_budget_k: Option<u64>,
    pub cumulative_tokens: u64,
    /// Redacted serialization of `ErrorInfo` (present only on terminal failure).
    pub error: Option<serde_json::Value>,
}

impl TraceEvent {
    /// Build a redacted `TraceEvent` from a progress update.
    pub fn from_progress(p: &SubagentProgress, session_id: &str) -> Self {
        // Parse `current_params` as JSON when possible so redaction can recurse
        // into structured params; otherwise keep the raw human-readable summary.
        let current_params = match &p.current_params {
            Some(s) => match serde_json::from_str::<serde_json::Value>(s) {
                Ok(v) => Some(redact_params(v)),
                Err(_) => Some(serde_json::Value::String(s.clone())),
            },
            None => None,
        };
        let status = serde_json::to_value(&p.status)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_default();
        // Redact the full ErrorInfo tree (covers failed_tool_sequence params,
        // retry_history, and failed_round_context) recursively.
        let error = p
            .error_details
            .as_ref()
            .map(|e| redact_params(serde_json::to_value(e).unwrap_or(serde_json::Value::Null)));
        Self {
            ts: chrono::Utc::now().timestamp_millis(),
            session_id: session_id.to_string(),
            node_id: p.node_id.clone(),
            parent_id: p.parent_id.clone(),
            label: p.label.clone(),
            status,
            round: p.round,
            current_tool: p.current_tool.clone(),
            current_params,
            elapsed_ms: p.elapsed_ms,
            progress_delta: p.progress_delta,
            token_budget_k: p.token_budget_k,
            cumulative_tokens: p.cumulative_tokens,
            error,
        }
    }
}

/// Async buffered JSONL trace sink.
///
/// Owns a writer task and exposes a `ProgressCallback` for wiring into the
/// subagent dispatch path (Task 3.2). Dropping without `shutdown` fires a
/// shutdown signal so the writer drains pending events and exits cleanly.
pub struct TraceSink {
    tx: Option<mpsc::Sender<TraceEvent>>,
    shutdown_tx: Option<oneshot::Sender<()>>,
    writer: Option<JoinHandle<std::io::Result<()>>>,
    callback: ProgressCallback,
}

impl TraceSink {
    /// Spawn the writer task and return a sink plus its callback.
    ///
    /// Must be called from a tokio runtime context (spawns a task).
    pub fn new(dir: PathBuf, session_id: impl Into<String>) -> Self {
        let session_id = session_id.into();
        let (tx, mut rx) = mpsc::channel::<TraceEvent>(TRACE_CHANNEL_CAPACITY);
        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

        let writer_dir = dir.clone();
        let writer_session = session_id.clone();
        let writer = tokio::spawn(async move {
            run_writer(&mut rx, shutdown_rx, &writer_dir, &writer_session).await
        });

        let callback: ProgressCallback = {
            let tx = tx.clone();
            let sid = session_id.clone();
            Arc::new(move |p: SubagentProgress| {
                let ev = TraceEvent::from_progress(&p, &sid);
                // Non-blocking: never stall the agent loop on disk I/O. A full
                // channel is a backpressure signal; we drop + warn rather than
                // block. Persistence is still best-effort here; the broadcast
                // channel's drop-oldest semantics are added in Task 3.3.
                if let Err(err) = tx.try_send(ev) {
                    tracing::warn!(
                        target: "wgenty::trace_sink",
                        error = %err,
                        "trace sink channel full or closed; dropping event"
                    );
                }
            })
        };

        Self {
            tx: Some(tx),
            shutdown_tx: Some(shutdown_tx),
            writer: Some(writer),
            callback,
        }
    }

    /// Return a clonable `ProgressCallback` that feeds this sink.
    pub fn callback(&self) -> ProgressCallback {
        Arc::clone(&self.callback)
    }

    /// Signal the writer to drain pending events and exit, then await it.
    pub async fn shutdown(mut self) -> std::io::Result<()> {
        // Signal the writer to drain + exit. Releasing our senders also lets
        // the channel close naturally, but the explicit signal guarantees the
        // writer wakes even if a callback clone lingers elsewhere.
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        // Drop our sender + callback clone so the channel can fully close.
        self.callback = Arc::new(|_| {});
        self.tx.take();
        if let Some(handle) = self.writer.take() {
            handle.await.map_err(std::io::Error::other)?
        } else {
            Ok(())
        }
    }
}

impl Drop for TraceSink {
    fn drop(&mut self) {
        // Best-effort: fire the shutdown signal so a detached writer still
        // drains pending events instead of lingering until all sender clones
        // (including the subagent loop's callback) are released.
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}

/// Writer task body: open the trace file and drain the channel in batches.
///
/// Exits on either an explicit shutdown signal (draining remaining events) or
/// when all senders drop (`recv` returns `None`).
async fn run_writer(
    rx: &mut mpsc::Receiver<TraceEvent>,
    shutdown_rx: oneshot::Receiver<()>,
    dir: &Path,
    session_id: &str,
) -> std::io::Result<()> {
    let mut file = open_trace_file(dir, session_id).await?;
    // Pin the shutdown future so it can be polled by reference across loop
    // iterations without being consumed.
    let mut shutdown = Box::pin(async {
        let _ = shutdown_rx.await;
    });
    loop {
        tokio::select! {
            biased;
            ev = rx.recv() => match ev {
                Some(ev) => {
                    write_event(&mut file, &ev).await?;
                    // Batch drain: coalesce a burst into a single flush.
                    while let Ok(ev) = rx.try_recv() {
                        write_event(&mut file, &ev).await?;
                    }
                    file.flush().await?;
                }
                None => {
                    while let Ok(ev) = rx.try_recv() {
                        write_event(&mut file, &ev).await?;
                    }
                    let _ = file.flush().await;
                    return Ok(());
                }
            },
            _ = &mut shutdown => {
                while let Ok(ev) = rx.try_recv() {
                    write_event(&mut file, &ev).await?;
                }
                let _ = file.flush().await;
                return Ok(());
            }
        }
    }
}

/// Serialize one event as a JSON line and append it.
async fn write_event(file: &mut tokio::fs::File, ev: &TraceEvent) -> std::io::Result<()> {
    let mut line = serde_json::to_vec(ev)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    line.push(b'\n');
    file.write_all(&line).await
}

/// Open (creating if needed) the trace JSONL file with secure permissions.
async fn open_trace_file(dir: &Path, session_id: &str) -> std::io::Result<tokio::fs::File> {
    tokio::fs::create_dir_all(dir).await?;
    secure_dir(dir).await;
    let fname = sanitize_session_id(session_id);
    let path = dir.join(format!("{fname}.jsonl"));
    let file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .await?;
    secure_file(&path).await;
    Ok(file)
}

/// Sanitize a session id for use as a filename (reject path separators / NUL).
fn sanitize_session_id(session_id: &str) -> String {
    session_id
        .chars()
        .map(|c| match c {
            '/' | '\\' | '\0' => '_',
            _ => c,
        })
        .collect()
}

#[cfg(unix)]
async fn secure_dir(dir: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = tokio::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).await;
}

#[cfg(not(unix))]
async fn secure_dir(_dir: &Path) {}

#[cfg(unix)]
async fn secure_file(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).await;
}

#[cfg(not(unix))]
async fn secure_file(_path: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::progress::{ErrorInfo, SubagentProgress, SubagentStatus};
    use crate::teams::failure_diagnostics::{FailureRootCause, ToolCallStep};
    use serde_json::{json, Value};
    use std::fs;
    use tempfile::TempDir;

    /// Minimal progress fixture; callers mutate fields as needed.
    fn make_progress() -> SubagentProgress {
        SubagentProgress {
            node_id: "agent-1".into(),
            parent_id: None,
            label: "explore src".into(),
            status: SubagentStatus::Running,
            round: Some(2),
            max_rounds: Some(10),
            current_tool: Some("file_read".into()),
            current_params: None,
            action_log: vec![],
            text_snapshot: None,
            started_at: 1_700_000_000_000,
            elapsed_ms: 1234,
            metadata: None,
            progress_delta: Some(0.25),
            token_budget_k: Some(10),
            cumulative_tokens: 2500,
            error_details: None,
            events: vec![],
            messages: vec![],
        }
    }

    #[test]
    fn test_trace_event_redacts_sensitive_current_params() {
        let mut p = make_progress();
        p.current_params = Some(r#"{"api_key":"sk-live","path":"/etc/passwd"}"#.into());

        let ev = TraceEvent::from_progress(&p, "sess-1");

        let params = ev.current_params.expect("params present");
        assert_eq!(
            params["api_key"], "***REDACTED***",
            "api_key must be redacted"
        );
        assert_eq!(
            params["path"], "/etc/passwd",
            "non-sensitive path must survive redaction"
        );
        assert!(ev.error.is_none());
    }

    #[test]
    fn test_trace_event_redacts_error_tool_sequence() {
        let mut p = make_progress();
        p.current_params = Some(r#"{"token":"abc","file":"x.rs"}"#.into());
        let err = ErrorInfo {
            root_cause: FailureRootCause::SandboxFailed,
            failed_tool_sequence: vec![ToolCallStep {
                tool_name: "exec_command".into(),
                params_summary: json!({"command": "ls", "env_token": "secret-val"}),
                elapsed_ms: 5,
            }],
            ..ErrorInfo::default()
        };
        p.error_details = Some(err);
        p.status = SubagentStatus::Failed;

        let ev = TraceEvent::from_progress(&p, "sess-1");
        let serialized = serde_json::to_string(&ev).unwrap();

        assert!(
            serialized.contains("***REDACTED***"),
            "redacted marker missing: {serialized}"
        );
        assert!(
            !serialized.contains("sk-live")
                && !serialized.contains("secret-val")
                && !serialized.contains("\"abc\""),
            "sensitive value leaked: {serialized}"
        );
        // root_cause must be present (snake_case serialized)
        assert!(
            serialized.contains("sandbox_failed"),
            "root cause missing: {serialized}"
        );
    }

    #[test]
    fn test_trace_event_non_json_params_kept_as_string() {
        let mut p = make_progress();
        p.current_params = Some("src/auth.rs:42".into());

        let ev = TraceEvent::from_progress(&p, "sess-1");
        match ev.current_params {
            Some(Value::String(s)) => assert_eq!(s, "src/auth.rs:42"),
            other => panic!("expected string param, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_trace_sink_appends_jsonl() {
        let dir = TempDir::new().unwrap();
        let sink = TraceSink::new(dir.path().to_path_buf(), "sess-1");
        let cb = sink.callback();

        let mut p1 = make_progress();
        p1.node_id = "agent-1".into();
        let mut p2 = make_progress();
        p2.node_id = "agent-1".into();
        p2.status = SubagentStatus::Completed;

        cb(p1);
        cb(p2);

        sink.shutdown().await.expect("shutdown ok");

        let path = dir.path().join("sess-1.jsonl");
        let content = fs::read_to_string(&path).expect("trace file readable");
        let lines: Vec<&str> = content.trim_end().lines().collect();
        assert_eq!(lines.len(), 2, "expected 2 events, got: {content}");

        let ev0: TraceEvent = serde_json::from_str(lines[0]).expect("line 0 valid json");
        let ev1: TraceEvent = serde_json::from_str(lines[1]).expect("line 1 valid json");
        assert_eq!(ev0.node_id, "agent-1");
        assert_eq!(ev0.session_id, "sess-1");
        assert_eq!(ev0.status, "Running");
        assert_eq!(ev1.status, "Completed");
    }

    #[tokio::test]
    async fn test_trace_sink_redacts_on_write() {
        let dir = TempDir::new().unwrap();
        let sink = TraceSink::new(dir.path().to_path_buf(), "sess-r");
        let cb = sink.callback();

        let mut p = make_progress();
        p.current_params = Some(r#"{"api_key":"sk-leak","path":"/tmp/x"}"#.into());

        cb(p);
        sink.shutdown().await.unwrap();

        let content = fs::read_to_string(dir.path().join("sess-r.jsonl")).unwrap();
        assert!(
            content.contains("***REDACTED***"),
            "redaction missing: {content}"
        );
        assert!(
            !content.contains("sk-leak"),
            "secret leaked to file: {content}"
        );
        assert!(content.contains("/tmp/x"), "benign path dropped: {content}");
    }

    #[tokio::test]
    async fn test_trace_sink_creates_missing_dir() {
        let dir = TempDir::new().unwrap();
        let nested = dir.path().join("deep").join("traces");
        let sink = TraceSink::new(nested.clone(), "sess-d");
        let cb = sink.callback();
        cb(make_progress());
        sink.shutdown().await.unwrap();

        let path = nested.join("sess-d.jsonl");
        assert!(
            path.exists(),
            "trace file should exist at {}",
            path.display()
        );
    }

    #[tokio::test]
    async fn test_trace_sink_shutdown_drains_pending() {
        let dir = TempDir::new().unwrap();
        let sink = TraceSink::new(dir.path().to_path_buf(), "sess-drain");
        let cb = sink.callback();

        // Fire several events then immediately shut down; all must persist.
        for i in 0..10 {
            let mut p = make_progress();
            p.node_id = format!("agent-{i}");
            cb(p);
        }
        sink.shutdown().await.unwrap();

        let content = fs::read_to_string(dir.path().join("sess-drain.jsonl")).unwrap();
        assert_eq!(
            content.trim_end().lines().count(),
            10,
            "all events must be drained"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_trace_sink_file_and_dir_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new().unwrap();
        let trace_dir = dir.path().join("traces");
        let sink = TraceSink::new(trace_dir.clone(), "sess-perm");
        let cb = sink.callback();
        cb(make_progress());
        sink.shutdown().await.unwrap();

        let dir_mode = fs::metadata(&trace_dir).unwrap().permissions().mode() & 0o777;
        assert_eq!(dir_mode, 0o700, "trace dir must be 0700, got {dir_mode:o}");

        let file_path = trace_dir.join("sess-perm.jsonl");
        let file_mode = fs::metadata(&file_path).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            file_mode, 0o600,
            "trace file must be 0600, got {file_mode:o}"
        );
    }
}
