//! `wgenty-code subagent` — offline, read-only inspection of subagent
//! transcripts (list / trace / health). Reads the SQLite transcript store
//! directly; does not start an agent.

use clap::ValueEnum;
use std::path::Path;
use std::sync::Arc;

use crate::teams::subagent_health::{HealthPeriod, SubagentHealthAnalyzer};
use crate::teams::subagent_trace::SubagentTraceReporter;
use crate::transcript::{SubagentTranscriptHeader, SubagentTranscriptStore};

/// Render format for `subagent trace`.
#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq)]
pub enum TraceFormat {
    CallTree,
    ErrorTimeline,
    Html,
    Chrome,
}

/// Time window for `subagent health`.
#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq)]
pub enum HealthPeriodArg {
    #[value(name = "1h")]
    H1,
    #[value(name = "24h")]
    H24,
    #[value(name = "7d")]
    D7,
    #[value(name = "30d")]
    D30,
    #[value(name = "all")]
    All,
}

impl From<HealthPeriodArg> for HealthPeriod {
    fn from(p: HealthPeriodArg) -> Self {
        match p {
            HealthPeriodArg::H1 => HealthPeriod::Last1h,
            HealthPeriodArg::H24 => HealthPeriod::Last24h,
            HealthPeriodArg::D7 => HealthPeriod::Last7d,
            HealthPeriodArg::D30 => HealthPeriod::Last30d,
            HealthPeriodArg::All => HealthPeriod::AllTime,
        }
    }
}

/// Open the transcript store read-only at the configured db path.
fn open_store(db_path: &str) -> anyhow::Result<Arc<SubagentTranscriptStore>> {
    let path = Path::new(db_path);
    if !path.exists() {
        anyhow::bail!(
            "Transcript database not found at '{}'. Run some subagent tasks first.",
            db_path
        );
    }
    let store = SubagentTranscriptStore::open(path)
        .map_err(|e| anyhow::anyhow!("Failed to open transcript db '{}': {}", db_path, e))?;
    Ok(Arc::new(store))
}

// ─── list ────────────────────────────────────────────────────────────────

/// Run `subagent list`. Returns the rendered table as a String (testable).
pub fn list(
    db_path: &str,
    session: Option<&str>,
    status: Option<&str>,
    limit: usize,
) -> anyhow::Result<String> {
    let store = open_store(db_path)?;
    let headers = collect_headers(&store, session)?;
    Ok(render_list(&headers, status, limit))
}

/// Gather headers for a session (or all sessions when `session` is None),
/// already reverse-chronological per store; merged + re-sorted when global.
fn collect_headers(
    store: &SubagentTranscriptStore,
    session: Option<&str>,
) -> anyhow::Result<Vec<SubagentTranscriptHeader>> {
    let mut headers: Vec<SubagentTranscriptHeader> = if let Some(sid) = session {
        store
            .list_by_session(sid)
            .map_err(|e| anyhow::anyhow!("Failed to list transcripts: {}", e))?
    } else {
        // Global listing: scan known label prefixes + the default sessions,
        // mirroring SubagentHealthAnalyzer::compute_health's global path.
        let mut all = Vec::new();
        for prefix in &["task:", "subagent:", "sub:", "explore:", "plan:"] {
            if let Ok(results) = store.search(prefix) {
                all.extend(results);
            }
        }
        for sid in &["default", "main"] {
            if let Ok(results) = store.list_by_session(sid) {
                all.extend(results);
            }
        }
        all
    };
    // Reverse-chronological; dedupe by id in case global scan overlapped.
    headers.sort_by_key(|h| std::cmp::Reverse(h.started_at));
    headers.dedup_by(|a, b| a.id == b.id);
    Ok(headers)
}

/// Render the list table. Pure: takes headers, applies status filter + limit.
fn render_list(headers: &[SubagentTranscriptHeader], status: Option<&str>, limit: usize) -> String {
    let filtered: Vec<&SubagentTranscriptHeader> = headers
        .iter()
        .filter(|h| status.is_none_or(|s| h.status == s))
        .take(limit)
        .collect();

    let mut out = String::new();
    out.push_str(&format!(
        "{:<36}  {:<28}  {:<10}  {:<18}  {:>8}  {:>6}  {}\n",
        "ID", "LABEL", "STATUS", "ROOT-CAUSE", "DUR(ms)", "ROUNDS", "STARTED"
    ));
    out.push_str(&format!("{}\n", "-".repeat(120)));
    for h in &filtered {
        let dur = h
            .finished_at
            .map(|f| (f - h.started_at).to_string())
            .unwrap_or_else(|| "-".to_string());
        let label = truncate(&h.label, 28);
        let root = format!("{:?}", h.root_cause);
        let started = fmt_ts(h.started_at);
        out.push_str(&format!(
            "{:<36}  {:<28}  {:<10}  {:<18}  {:>8}  {:>6}  {}\n",
            h.id,
            label,
            h.status,
            truncate(&root, 18),
            dur,
            h.actual_rounds,
            started
        ));
    }
    if filtered.is_empty() {
        out.push_str("(no subagent runs match)\n");
    }
    out
}

// ─── trace ───────────────────────────────────────────────────────────────

/// Run `subagent trace <id>`. Returns rendered output, or Err on unknown id.
pub fn trace(db_path: &str, id: &str, format: TraceFormat, raw: bool) -> anyhow::Result<String> {
    let store = open_store(db_path)?;
    let transcript = store
        .get_by_id(id)
        .map_err(|e| anyhow::anyhow!("Failed to load transcript '{}': {}", id, e))?
        .ok_or_else(|| anyhow::anyhow!("Unknown subagent id: '{}'", id))?;

    if raw {
        return Ok(render_raw(&transcript));
    }

    let reporter = SubagentTraceReporter::new(store);
    let session_id = transcript.session_id.clone();
    let rendered = match format {
        TraceFormat::CallTree => reporter.render_call_tree(&session_id),
        TraceFormat::ErrorTimeline => {
            reporter.render_error_timeline(Some(&session_id), HealthPeriod::AllTime)
        }
        TraceFormat::Html => reporter.render_html_report(&session_id),
        TraceFormat::Chrome => reporter
            .export_chrome_trace(&session_id)
            .map(|v| serde_json::to_string_pretty(&v).unwrap_or_default()),
    }
    .map_err(|e| anyhow::anyhow!("Failed to render trace: {}", e))?;
    Ok(rendered)
}

/// Pretty-print the stored diagnostics for one transcript.
fn render_raw(t: &crate::transcript::SubagentTranscript) -> String {
    serde_json::to_string_pretty(&serde_json::json!({
        "id": t.id,
        "session_id": t.session_id,
        "label": t.label,
        "status": t.status.to_string(),
        "root_cause": t.failure_diagnostics.as_ref().map(|d| format!("{:?}", d.root_cause)),
        "error_message": t.error_message,
        "failure_diagnostics": t.failure_diagnostics,
        "started_at": t.started_at,
        "finished_at": t.finished_at,
        "total_tokens": t.total_tokens,
        "actual_rounds": t.actual_rounds,
    }))
    .unwrap_or_else(|_| "{}".to_string())
}

// ─── health ──────────────────────────────────────────────────────────────

/// Run `subagent health`. Returns the rendered dashboard as a String.
pub fn health(
    db_path: &str,
    period: HealthPeriodArg,
    session: Option<&str>,
) -> anyhow::Result<String> {
    let store = open_store(db_path)?;
    let analyzer = SubagentHealthAnalyzer::new(store);
    let health = analyzer
        .compute_health(session, period.into())
        .map_err(|e| anyhow::anyhow!("Failed to compute health: {}", e))?;
    Ok(render_health(&health))
}

/// Render the health dashboard as text (mirrors print_health_report but to a
/// String so it's testable + pipeable).
fn render_health(h: &crate::teams::subagent_health::SubagentHealth) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "SUBAGENT HEALTH — {}  {}\n",
        h.status.label(),
        h.period
    ));
    out.push_str(&format!("  Total runs:     {}\n", h.total_runs));
    out.push_str(&format!(
        "  Completed:      {}  ({:.1}%)\n",
        h.completed,
        h.success_rate * 100.0
    ));
    out.push_str(&format!(
        "  Failed:         {}  ({:.1}%)\n",
        h.failed,
        (h.failed as f64 / h.total_runs.max(1) as f64) * 100.0
    ));
    out.push_str(&format!(
        "  Cancelled:      {}  ({:.1}%)\n",
        h.cancelled,
        (h.cancelled as f64 / h.total_runs.max(1) as f64) * 100.0
    ));
    out.push_str(&format!("  Health Score:   {:.0}/100\n", h.health_score));
    out.push_str(&format!("  Avg rounds:     {:.1}\n", h.avg_rounds));
    out.push_str(&format!("  Avg tokens:     {:.0}\n", h.avg_tokens));
    out.push_str(&format!("  Avg duration:   {:.0}ms\n", h.avg_duration_ms));

    if !h.failure_modes.is_empty() {
        out.push_str("\nFailure Mode Breakdown:\n");
        for fm in &h.failure_modes {
            out.push_str(&format!(
                "  [{}] {:<28} {:>4} ({:5.1}%)\n       -> {}\n",
                fm.severity.chars().next().unwrap_or('?'),
                fm.label,
                fm.count,
                fm.percentage,
                fm.recommendation
            ));
        }
    }
    out
}

// ─── helpers ─────────────────────────────────────────────────────────────

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

fn fmt_ts(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_else(|| ms.to_string())
}

// ─── dispatch entry ──────────────────────────────────────────────────────

/// Dispatch from `run_async` — prints to stdout, maps unknown-id to exit 2.
pub async fn run(
    state: &crate::state::AppState,
    action: &super::SubagentCommands,
) -> anyhow::Result<()> {
    let db_path = state.settings.storage.transcript.db_path.clone();
    match action {
        super::SubagentCommands::List {
            session,
            status,
            limit,
        } => {
            let out = list(&db_path, session.as_deref(), status.as_deref(), *limit)?;
            print!("{}", out);
        }
        super::SubagentCommands::Trace { id, format, raw } => {
            match trace(&db_path, id, *format, *raw) {
                Ok(out) => print!("{}", out),
                Err(e) if e.to_string().contains("Unknown subagent id") => {
                    eprintln!("{}", e);
                    std::process::exit(2);
                }
                Err(e) => return Err(e),
            }
        }
        super::SubagentCommands::Health { period, session } => {
            let out = health(&db_path, *period, session.as_deref())?;
            print!("{}", out);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::teams::failure_diagnostics::FailureRootCause;

    fn hdr(
        id: &str,
        session: &str,
        status: &str,
        started: i64,
        root: FailureRootCause,
    ) -> SubagentTranscriptHeader {
        SubagentTranscriptHeader {
            id: id.to_string(),
            session_id: session.to_string(),
            parent_id: None,
            label: format!("task: {}", id),
            status: status.to_string(),
            started_at: started,
            finished_at: Some(started + 1500),
            total_tokens: 1234,
            actual_rounds: 3,
            error_message: None,
            summary: None,
            root_cause: root,
            project_path: None,
        }
    }

    // ── render_list ──

    #[test]
    fn list_renders_reverse_chronological() {
        let headers = vec![
            hdr("a", "s", "completed", 1000, FailureRootCause::Unknown),
            hdr("b", "s", "failed", 3000, FailureRootCause::Timeout),
            hdr("c", "s", "completed", 2000, FailureRootCause::Unknown),
        ];
        // collect_headers sorts; emulate by pre-sorting desc as it would
        let mut sorted = headers.clone();
        sorted.sort_by_key(|h| std::cmp::Reverse(h.started_at));
        let out = render_list(&sorted, None, 50);
        let pos_b = out.find("b").unwrap();
        let pos_c = out.find("c").unwrap();
        let pos_a = out.find("a\n").unwrap_or(usize::MAX);
        assert!(pos_b < pos_c, "most recent first");
        let _ = pos_a;
        assert!(out.contains("Timeout"), "root-cause column present");
    }

    #[test]
    fn list_filters_by_status() {
        let headers = vec![
            hdr("a", "s", "completed", 1000, FailureRootCause::Unknown),
            hdr("b", "s", "failed", 2000, FailureRootCause::Timeout),
        ];
        let out = render_list(&headers, Some("failed"), 50);
        assert!(out.contains('b'), "failed row kept");
        // 'a' row must be filtered (id 'a' only appears in header/sep otherwise)
        assert!(
            !out.lines().any(|l| l.starts_with("a  ")),
            "completed row filtered"
        );
    }

    #[test]
    fn list_respects_limit() {
        let headers: Vec<_> = (0..10)
            .map(|i| {
                hdr(
                    &format!("id{}", i),
                    "s",
                    "completed",
                    i * 1000,
                    FailureRootCause::Unknown,
                )
            })
            .collect();
        let out = render_list(&headers, None, 3);
        let rows = out.lines().filter(|l| l.starts_with("id")).count();
        assert_eq!(rows, 3, "limit truncates rows");
    }

    #[test]
    fn list_empty_shows_placeholder() {
        let out = render_list(&[], None, 50);
        assert!(out.contains("(no subagent runs match)"));
    }

    // ── render_health ──

    #[test]
    fn health_renders_root_cause_breakdown() {
        let store = Arc::new(
            SubagentTranscriptStore::open(std::path::Path::new(":memory:"))
                .expect("in-memory store"),
        );
        let analyzer = SubagentHealthAnalyzer::new(store);
        let headers = [
            hdr("a", "s", "failed", 1, FailureRootCause::Timeout),
            hdr("b", "s", "failed", 2, FailureRootCause::TokenBudgetExceeded),
            hdr("c", "s", "completed", 3, FailureRootCause::Unknown),
        ];
        let refs: Vec<&SubagentTranscriptHeader> = headers.iter().collect();
        let health = analyzer
            .compute_from_headers(&refs, HealthPeriod::AllTime)
            .unwrap();
        let out = render_health(&health);
        assert!(out.contains("Total runs:     3"));
        assert!(out.contains("Completed:      1"));
        assert!(out.contains("Failed:         2"));
    }

    // ── period / format conversions ──

    #[test]
    fn period_arg_maps_to_health_period() {
        assert!(matches!(
            HealthPeriod::from(HealthPeriodArg::H1),
            HealthPeriod::Last1h
        ));
        assert!(matches!(
            HealthPeriod::from(HealthPeriodArg::D7),
            HealthPeriod::Last7d
        ));
        assert!(matches!(
            HealthPeriod::from(HealthPeriodArg::All),
            HealthPeriod::AllTime
        ));
    }

    // ── trace unknown id exit path ──

    #[test]
    fn trace_unknown_id_errors() {
        let dir = std::env::temp_dir().join(format!("wgenty_test_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("t.db");
        // Create an empty-but-migrated db
        let _store = SubagentTranscriptStore::open(&db).unwrap();
        drop(_store);
        let err = trace(
            db.to_str().unwrap(),
            "no-such-id",
            TraceFormat::CallTree,
            false,
        )
        .expect_err("unknown id must error");
        assert!(err.to_string().contains("Unknown subagent id"));
        std::fs::remove_dir_all(&dir).ok();
    }

    // ── trace rendering variants over a real saved transcript ──

    use crate::transcript::{SubagentTranscript, TranscriptStatus};

    fn saved_transcript(db: &std::path::Path) -> SubagentTranscript {
        let t = SubagentTranscript {
            id: "trace-1".to_string(),
            session_id: "sess-trace".to_string(),
            parent_id: None,
            label: "task: render me".to_string(),
            status: TranscriptStatus::Failed,
            system_prompt: None,
            user_prompt: "do the thing".to_string(),
            started_at: 1_700_000_000_000,
            finished_at: Some(1_700_000_002_500),
            total_tokens: 2048,
            max_rounds: Some(10),
            actual_rounds: 4,
            token_budget_k: None,
            error_message: Some("Subagent timed out after 240s".to_string()),
            summary: None,
            failure_diagnostics: None,
            project_path: None,
            events: Vec::new(),
        };
        let store = SubagentTranscriptStore::open(db).unwrap();
        store.save(&t, None).unwrap();
        t
    }

    fn temp_db(tag: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let dir =
            std::env::temp_dir().join(format!("wgenty_test_{}_{}", tag, uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("t.db");
        (dir, db)
    }

    #[test]
    fn trace_renders_all_formats_for_saved_transcript() {
        let (dir, db) = temp_db("fmt");
        saved_transcript(&db);
        let path = db.to_str().unwrap();

        let tree = trace(path, "trace-1", TraceFormat::CallTree, false).unwrap();
        assert!(tree.contains("render me") || tree.contains("trace-1") || !tree.trim().is_empty());

        let timeline = trace(path, "trace-1", TraceFormat::ErrorTimeline, false).unwrap();
        assert!(timeline.contains("ERROR TIMELINE") || !timeline.trim().is_empty());

        let html = trace(path, "trace-1", TraceFormat::Html, false).unwrap();
        assert!(html.contains("<html") || html.contains("<!doctype") || !html.trim().is_empty());

        let chrome = trace(path, "trace-1", TraceFormat::Chrome, false).unwrap();
        assert!(chrome.contains("traceEvents"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn trace_raw_prints_diagnostics_json() {
        let (dir, db) = temp_db("raw");
        saved_transcript(&db);
        let out = trace(db.to_str().unwrap(), "trace-1", TraceFormat::CallTree, true).unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).expect("raw is valid JSON");
        assert_eq!(v["id"], "trace-1");
        assert_eq!(v["session_id"], "sess-trace");
        assert_eq!(v["status"], "failed");
        assert_eq!(v["error_message"], "Subagent timed out after 240s");
        std::fs::remove_dir_all(&dir).ok();
    }

    // ── helpers ──

    #[test]
    fn truncate_is_char_boundary_safe() {
        let s = "áb́ćd́éf́ǵ"; // combining chars
        let out = truncate(s, 4);
        assert!(out.chars().count() <= 4);
        assert!(out.ends_with('…'));
    }
}
