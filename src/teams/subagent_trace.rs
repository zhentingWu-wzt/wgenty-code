//! Subagent Trace Reporter — call tree visualization, error timeline,
//! and Chrome Trace Event Format export.
//!
//! Queries the SubagentTranscriptStore to build a tree of subagent
//! executions with their tool calls, then renders them as ASCII art
//! or exports JSON for external tools (Perfetto, Chrome DevTools).

use crate::teams::subagent_health::{HealthPeriod, SubagentHealthAnalyzer};
use crate::transcript::{SubagentTranscriptHeader, SubagentTranscriptStore};
use std::collections::HashMap;
use std::sync::Arc;

// ─── Trace data model ──────────────────────────────────────────────────────

/// A node in the subagent call tree, enriched with timing and tool events.
#[derive(Debug, Clone)]
// Fields are populated during trace collection; some are only
// read by future rendering formats (JSON export, tree view).
#[allow(dead_code)]
struct TraceNode {
    id: String,
    label: String,
    status: String,
    started_at: i64,
    finished_at: Option<i64>,
    error_message: Option<String>,
    total_tokens: u64,
    actual_rounds: u32,
    summary: Option<String>,
    events: Vec<TraceEvent>,
    children: Vec<TraceNode>,
}

impl TraceNode {
    fn duration_ms(&self) -> u64 {
        match self.finished_at {
            Some(finished) if finished > self.started_at => (finished - self.started_at) as u64,
            _ => 0,
        }
    }

    fn is_success(&self) -> bool {
        self.status == "completed"
    }

    fn is_failed(&self) -> bool {
        self.status == "failed"
    }

    fn is_cancelled(&self) -> bool {
        self.status == "cancelled"
    }

    fn status_icon(&self) -> &'static str {
        if self.is_success() {
            "\u{2705}" // ✅
        } else if self.is_failed() {
            "\u{274c}" // ❌
        } else if self.is_cancelled() {
            "\u{1f6ab}" // 🚫
        } else {
            "\u{23f3}" // ⏳
        }
    }
}

#[derive(Debug, Clone)]
// Populated for completeness; not all fields are rendered yet.
#[allow(dead_code)]
struct TraceEvent {
    round: u32,
    event_type: String,
    tool_name: Option<String>,
    tool_params: Option<String>,
    elapsed_ms: u64,
    data: String,
}

// ─── Reporter ──────────────────────────────────────────────────────────────

/// Reporter that renders subagent traces in multiple formats.
pub struct SubagentTraceReporter {
    store: Arc<SubagentTranscriptStore>,
}

impl SubagentTraceReporter {
    pub fn new(store: Arc<SubagentTranscriptStore>) -> Self {
        Self { store }
    }

    // ─── Tree builder ──────────────────────────────────────────────────────

    /// Build the full trace tree for a session from the transcript store.
    fn build_trace_tree(&self, session_id: &str) -> Result<Vec<TraceNode>, String> {
        let headers = self
            .store
            .list_by_session(session_id)
            .map_err(|e| format!("Failed to list transcripts: {}", e))?;

        if headers.is_empty() {
            return Err(format!(
                "No subagent data found for session '{}'",
                session_id
            ));
        }

        // Load full transcripts with events
        let mut nodes: Vec<TraceNode> = Vec::new();
        for header in &headers {
            let transcript = self
                .store
                .get_by_id(&header.id)
                .map_err(|e| format!("Failed to load transcript {}: {}", header.id, e))?;

            if let Some(t) = transcript {
                nodes.push(TraceNode {
                    id: t.id,
                    label: t.label,
                    status: t.status.to_string(),
                    started_at: t.started_at,
                    finished_at: t.finished_at,
                    error_message: t.error_message,
                    total_tokens: t.total_tokens,
                    actual_rounds: t.actual_rounds,
                    summary: t.summary,
                    events: t
                        .events
                        .iter()
                        .map(|e| TraceEvent {
                            round: e.round,
                            event_type: e.event_type.clone(),
                            tool_name: e.tool_name.clone(),
                            tool_params: e.tool_params.as_ref().map(|v| {
                                let s = v.to_string();
                                safe_truncate(&s, 60)
                            }),
                            elapsed_ms: e.elapsed_ms,
                            data: e.data.clone(),
                        })
                        .collect(),
                    children: Vec::new(),
                });
            }
        }

        // Build parent-child relationships
        let mut node_map: HashMap<String, usize> = HashMap::new();
        let mut roots: Vec<TraceNode> = Vec::new();

        for (i, node) in nodes.iter().enumerate() {
            node_map.insert(node.id.clone(), i);
        }

        // Collect parent_id info from headers
        let mut children_map: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, header) in headers.iter().enumerate() {
            if let Some(ref parent_id) = header.parent_id {
                children_map.entry(parent_id.clone()).or_default().push(i);
            }
        }

        // Build tree structure
        let mut processed = vec![false; nodes.len()];
        for i in 0..nodes.len() {
            if processed[i] {
                continue;
            }
            let pid = headers[i].parent_id.clone();
            if pid.is_none() || !node_map.contains_key(pid.as_ref().unwrap()) {
                // Root node
                processed[i] = true;
                attach_children(&mut nodes, &children_map, i, &mut processed);
                roots.push(nodes[i].clone());
            }
        }

        // Any remaining unprocessed nodes (orphans) become roots
        for i in 0..nodes.len() {
            if !processed[i] {
                processed[i] = true;
                roots.push(nodes[i].clone());
            }
        }

        // Sort by start time
        roots.sort_by_key(|n| n.started_at);

        Ok(roots)
    }

    // ─── ASCII Call Tree + Waterfall renderer ───────────────────────────────

    /// Render a session's subagent trace as an ASCII call tree with waterfall bars.
    pub fn render_call_tree(&self, session_id: &str) -> Result<String, String> {
        let roots = self.build_trace_tree(session_id)?;
        if roots.is_empty() {
            return Ok("No subagent activity recorded for this session.".to_string());
        }

        // Compute global time bounds for the waterfall
        let min_ts = roots.iter().map(min_start_ts).min().unwrap_or(0);
        let max_ts = roots.iter().map(max_end_ts).max().unwrap_or(0);
        let total_span_ms = (max_ts - min_ts).max(1) as u64;
        let bar_width = 50usize;

        let mut buf = String::new();
        let session_start = format_time(min_ts);
        let session_end = format_time(max_ts);
        buf.push_str(&format!(
            "\nSUBAGENT TRACE  (session: {} → {})\n\n",
            session_start, session_end
        ));

        let mut is_first = true;
        for root in &roots {
            if !is_first {
                buf.push('\n');
            }
            is_first = false;
            self.render_node(&mut buf, root, "", true, min_ts, total_span_ms, bar_width);
        }

        buf.push_str(
            "\n─ Legend ──────────────────────────────────────────────\n\
         ✅ completed   ❌ failed   🚫 cancelled   ⏳ running\n\
         ├─ tool_name(params) = tool call in waterfall\n",
        );
        Ok(buf)
    }

    #[allow(clippy::too_many_arguments)]
    fn render_node(
        &self,
        buf: &mut String,
        node: &TraceNode,
        prefix: &str,
        is_last: bool,
        min_ts: i64,
        total_span_ms: u64,
        bar_width: usize,
    ) {
        let connector = if is_last { "└─ " } else { "├─ " };
        let child_prefix = if is_last { "   " } else { "│  " };

        // Status icon + label + duration
        let duration_ms = node.duration_ms();
        let dur_str = format_duration(duration_ms);
        let icon = node.status_icon();

        // Waterfall bar
        let bar = waterfall_bar(
            node.started_at,
            node.finished_at,
            min_ts,
            total_span_ms,
            bar_width,
        );

        buf.push_str(&format!(
            "{}[{}] {} {} {}\n",
            prefix, icon, node.label, bar, dur_str
        ));

        // Error info
        if let Some(ref err) = node.error_message {
            let short_err = safe_truncate(err, 100);
            buf.push_str(&format!(
                "{} {}{}{} Error: {}\n",
                prefix,
                child_prefix,
                connector,
                if is_last { "  " } else { "│ " },
                short_err
            ));
        }

        // Render tool calls as sub-items (only the most significant ones)
        let tool_events: Vec<&TraceEvent> = node
            .events
            .iter()
            .filter(|e| e.event_type == "tool_result" || e.event_type == "action")
            .collect();

        for (i, event) in tool_events.iter().enumerate() {
            let is_last_tool = i == tool_events.len() - 1 && node.children.is_empty();
            let tool_connector = if is_last_tool { "└─ " } else { "├─ " };
            let tool_dur = format_duration(event.elapsed_ms);
            let tool_name = event.tool_name.as_deref().unwrap_or("unknown");
            let params = event.tool_params.as_deref().unwrap_or("");
            let tool_icon = if event.event_type == "tool_result" {
                if event.data.contains("Error")
                    || event.data.contains("error")
                    || event.data.contains("fail")
                {
                    "❌"
                } else {
                    "│ "
                }
            } else {
                "│ "
            };

            buf.push_str(&format!(
                "{} {}{} {} {}({}) — {}\n",
                prefix, child_prefix, tool_connector, tool_icon, tool_name, params, tool_dur
            ));
        }

        // Render children
        for (i, child) in node.children.iter().enumerate() {
            let child_is_last = i == node.children.len() - 1;
            self.render_node(
                buf,
                child,
                &format!("{}{}", prefix, child_prefix),
                child_is_last,
                min_ts,
                total_span_ms,
                bar_width,
            );
        }
    }

    // ─── Error timeline renderer ──────────────────────────────────────────

    /// Render an error timeline for subagents in the given period.
    pub fn render_error_timeline(
        &self,
        session_id: Option<&str>,
        period: HealthPeriod,
    ) -> Result<String, String> {
        let analyzer = SubagentHealthAnalyzer::new(self.store.clone());
        let health = analyzer.compute_health(session_id, period)?;

        let mut buf = String::new();
        buf.push_str(&format!(
            "\nSUBAGENT ERROR TIMELINE  ({})\n\n",
            health.period
        ));

        if health.total_runs == 0 {
            buf.push_str("  No subagent activity recorded in this period.\n");
            return Ok(buf);
        }

        // Summary line
        buf.push_str(&format!(
            "  {} {} total runs | {} completed | {} failed | {} cancelled | Health: {}\n\n",
            health.status.label(),
            health.total_runs,
            health.completed,
            health.failed,
            health.cancelled,
            health.status.label(),
        ));

        if health.failure_modes.is_empty() {
            buf.push_str("  No failures detected. 🎉\n");
        } else {
            buf.push_str("┌─ Failure Mode Breakdown ─────────────────────────────────────\n");
            for fm in &health.failure_modes {
                let severity_marker = match fm.severity.as_str() {
                    "Critical" => "🔴",
                    "Warning" => "🟡",
                    _ => "🔵",
                };
                let bar = "█".repeat((fm.percentage / 2.0) as usize);
                buf.push_str(&format!(
                    "│ {} {:30} {:>4} ({:5.1}%) {}\n",
                    severity_marker, fm.label, fm.count, fm.percentage, bar
                ));
                buf.push_str(&format!("│    → {}\n", fm.recommendation));
            }
            buf.push_str("└───────────────────────────────────────────────────────────────\n");
        }

        // Per-subagent error list
        if let Some(sid) = session_id {
            let headers = self
                .store
                .list_by_session(sid)
                .map_err(|e| format!("Failed to list transcripts: {}", e))?;

            let failures: Vec<&SubagentTranscriptHeader> = headers
                .iter()
                .filter(|h| h.status == "failed" || h.status == "cancelled")
                .collect();

            if !failures.is_empty() {
                buf.push_str("\n┌─ Error Details ──────────────────────────────────────────────\n");
                for h in failures {
                    let time = format_time(h.started_at);
                    let status_icon = if h.status == "cancelled" {
                        "🚫"
                    } else {
                        "❌"
                    };
                    let err_msg = h.error_message.as_deref().unwrap_or("(no details)");
                    let err_short = safe_truncate(err_msg, 80);
                    buf.push_str(&format!(
                        "│ {} {} {} — rounds={}, tokens={}\n│   {}\n",
                        status_icon, time, h.label, h.actual_rounds, h.total_tokens, err_short
                    ));
                }
                buf.push_str("└───────────────────────────────────────────────────────────────\n");
            }
        }

        Ok(buf)
    }

    // ─── Chrome Trace Event Format export ─────────────────────────────────

    /// Export a session's subagent trace as Chrome Trace Event Format JSON.
    /// This can be loaded into `ui.perfetto.dev` or Chrome DevTools Performance tab
    /// for visual waterfall/flamegraph analysis.
    pub fn export_chrome_trace(&self, session_id: &str) -> Result<serde_json::Value, String> {
        let roots = self.build_trace_tree(session_id)?;

        let mut events: Vec<serde_json::Value> = Vec::new();
        let process_id: i64 = 0;
        let mut next_thread_id: i64 = 1;

        // Assign thread IDs to each node
        let mut thread_map: HashMap<String, i64> = HashMap::new();
        assign_thread_ids(&roots, &mut thread_map, &mut next_thread_id);

        // Generate B/E events for each node
        collect_chrome_events(&roots, &mut events, &thread_map, process_id);

        // Sort by timestamp
        events.sort_by(|a, b| {
            let ta = a["ts"].as_i64().unwrap_or(0);
            let tb = b["ts"].as_i64().unwrap_or(0);
            ta.cmp(&tb)
        });

        Ok(serde_json::json!({
            "displayTimeUnit": "ms",
            "traceEvents": events,
        }))
    }

    // ─── HTML Report renderer (Layer 3) ───────────────────────────────────

    /// Render a self-contained HTML report with collapsible call tree,
    /// event timeline, and health dashboard. No external dependencies.
    pub fn render_html_report(&self, session_id: &str) -> Result<String, String> {
        let roots = self.build_trace_tree(session_id)?;
        let analyzer = SubagentHealthAnalyzer::new(self.store.clone());
        let health = analyzer.compute_health(Some(session_id), HealthPeriod::AllTime)?;

        // Serialize data for embedding
        let tree_json = nodes_to_json(&roots);
        let health_json = serde_json::json!({
            "period": health.period,
            "total_runs": health.total_runs,
            "completed": health.completed,
            "failed": health.failed,
            "cancelled": health.cancelled,
            "success_rate": health.success_rate,
            "avg_rounds": health.avg_rounds,
            "avg_tokens": health.avg_tokens,
            "avg_duration_ms": health.avg_duration_ms,
            "health_score": health.health_score,
            "status": health.status.label(),
            "failure_modes": health.failure_modes.iter().map(|fm| serde_json::json!({
                "label": fm.label,
                "count": fm.count,
                "percentage": fm.percentage,
                "severity": fm.severity,
                "recommendation": fm.recommendation,
            })).collect::<Vec<_>>(),
            "recommendations": health.recommendations,
        });

        let html = build_html_report(&tree_json, &health_json, session_id);
        Ok(html)
    }
}

// ─── Standalone functions ──────────────────────────────────────────────

fn assign_thread_ids(
    nodes: &[TraceNode],
    thread_map: &mut HashMap<String, i64>,
    next_id: &mut i64,
) {
    for node in nodes {
        thread_map.entry(node.id.clone()).or_insert_with(|| {
            let id = *next_id;
            *next_id += 1;
            id
        });
        assign_thread_ids(&node.children, thread_map, next_id);
    }
}

fn collect_chrome_events(
    nodes: &[TraceNode],
    events: &mut Vec<serde_json::Value>,
    thread_map: &HashMap<String, i64>,
    process_id: i64,
) {
    for node in nodes {
        let tid = thread_map.get(&node.id).copied().unwrap_or(0);

        // Begin event
        events.push(serde_json::json!({
            "pid": process_id,
            "tid": tid,
            "ph": "B",
            "name": node.label,
            "ts": node.started_at * 1000, // ms → μs
            "args": {
                "id": node.id,
                "status": node.status,
                "rounds": node.actual_rounds,
                "tokens": node.total_tokens,
            }
        }));

        // Tool call sub-events
        for event in &node.events {
            if event.event_type == "action" || event.event_type == "tool_result" {
                let tool_name = event.tool_name.as_deref().unwrap_or("unknown");
                let tool_ts = node.started_at + event.elapsed_ms as i64;
                events.push(serde_json::json!({
                    "pid": process_id,
                    "tid": tid,
                    "ph": "X",
                    "name": tool_name,
                    "ts": tool_ts * 1000,
                    "dur": 100, // 100μs marker dot
                    "args": {
                        "params": event.tool_params,
                        "type": event.event_type,
                    }
                }));
            }
        }

        // End event
        if let Some(finished) = node.finished_at {
            events.push(serde_json::json!({
                "pid": process_id,
                "tid": tid,
                "ph": "E",
                "name": node.label,
                "ts": finished * 1000,
            }));
        } else {
            // Still running — use now as end
            let now = chrono::Utc::now().timestamp_millis();
            events.push(serde_json::json!({
                "pid": process_id,
                "tid": tid,
                "ph": "E",
                "name": node.label,
                "ts": now * 1000,
            }));
        }

        collect_chrome_events(&node.children, events, thread_map, process_id);
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────────

/// Truncate a string for display, ensuring the cut lands on a valid UTF-8
/// character boundary. Returns `s` unchanged if shorter than `max_len`.
fn safe_truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        return s.to_string();
    }
    let mut end = max_len;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    if end == 0 {
        return String::new();
    }
    format!("{}…", &s[..end])
}

/// Format a unix millisecond timestamp as HH:MM:SS
fn format_time(ts_ms: i64) -> String {
    use chrono::TimeZone;
    let dt = chrono::Utc.timestamp_millis_opt(ts_ms).unwrap();
    dt.format("%H:%M:%S").to_string()
}

/// Format duration in human-readable form
fn format_duration(ms: u64) -> String {
    if ms >= 60_000 {
        format!("{:.1}m", ms as f64 / 60_000.0)
    } else if ms >= 1_000 {
        format!("{:.1}s", ms as f64 / 1_000.0)
    } else {
        format!("{}ms", ms)
    }
}

/// Build a waterfall bar string showing the node's time span
fn waterfall_bar(
    started_at: i64,
    finished_at: Option<i64>,
    min_ts: i64,
    total_span_ms: u64,
    bar_width: usize,
) -> String {
    let end_ts = finished_at.unwrap_or_else(|| chrono::Utc::now().timestamp_millis());
    let offset_ms = (started_at - min_ts).max(0) as u64;
    let duration_ms = (end_ts - started_at).max(0) as u64;

    let start_pos = ((offset_ms as f64 / total_span_ms.max(1) as f64) * bar_width as f64) as usize;
    let bar_len =
        ((duration_ms as f64 / total_span_ms.max(1) as f64) * bar_width as f64).max(1.0) as usize;

    let mut bar = String::with_capacity(bar_width + 2);
    for i in 0..bar_width {
        if i >= start_pos && i < start_pos + bar_len {
            bar.push('─');
        } else {
            bar.push(' ');
        }
    }

    // Add duration label at end
    format!("│{}│", bar)
}

/// Get the earliest start time in a tree
fn min_start_ts(node: &TraceNode) -> i64 {
    let mut min = node.started_at;
    for child in &node.children {
        let child_min = min_start_ts(child);
        if child_min < min {
            min = child_min;
        }
    }
    min
}

/// Get the latest end time in a tree
fn max_end_ts(node: &TraceNode) -> i64 {
    let mut max = node
        .finished_at
        .unwrap_or_else(|| chrono::Utc::now().timestamp_millis());
    for child in &node.children {
        let child_max = max_end_ts(child);
        if child_max > max {
            max = child_max;
        }
    }
    max
}

// ─── JSON tree serializer ──────────────────────────────────────────────

/// Serialize a `TraceNode` tree into `serde_json::Value` for embedding in
/// HTML reports. Preserves full tree structure including children and events.
fn nodes_to_json(nodes: &[TraceNode]) -> serde_json::Value {
    let arr: Vec<serde_json::Value> = nodes
        .iter()
        .map(|node| {
            serde_json::json!({
                "id": node.id,
                "label": node.label,
                "status": node.status,
                "started_at": node.started_at,
                "finished_at": node.finished_at,
                "error_message": node.error_message,
                "total_tokens": node.total_tokens,
                "actual_rounds": node.actual_rounds,
                "summary": node.summary,
                "duration_ms": node.duration_ms(),
                "is_success": node.is_success(),
                "is_failed": node.is_failed(),
                "is_cancelled": node.is_cancelled(),
                "events": node.events.iter().map(|e| serde_json::json!({
                    "round": e.round,
                    "event_type": e.event_type,
                    "tool_name": e.tool_name,
                    "tool_params": e.tool_params,
                    "elapsed_ms": e.elapsed_ms,
                    "data": e.data,
                })).collect::<Vec<_>>(),
                "children": nodes_to_json(&node.children),
            })
        })
        .collect();
    serde_json::Value::Array(arr)
}

// ─── HTML Report builder ──────────────────────────────────────────────

/// Build a self-contained HTML report from subagent trace and health data.
/// All CSS and JS are inlined — no external dependencies.
fn build_html_report(
    tree_json: &serde_json::Value,
    health_json: &serde_json::Value,
    session_id: &str,
) -> String {
    let tree_str = serde_json::to_string(tree_json).unwrap_or_else(|_| "[]".to_string());
    let health_str = serde_json::to_string(health_json).unwrap_or_else(|_| "{}".to_string());

    format!(
        r##"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>Subagent Trace — {session_id}</title>
<style>
/* === Catppuccin Mocha Theme === */
:root {{
  --base: #1e1e2e;
  --mantle: #181825;
  --crust: #11111b;
  --surface0: #313244;
  --surface1: #45475a;
  --text: #cdd6f4;
  --subtext0: #a6adc8;
  --subtext1: #bac2de;
  --overlay0: #6c7086;
  --green: #a6e3a1;
  --red: #f38ba8;
  --yellow: #f9e2af;
  --blue: #89b4fa;
  --lavender: #b4befe;
  --peach: #fab387;
  --teal: #94e2d5;
  --mauve: #cba6f7;
}}
* {{ box-sizing: border-box; margin: 0; padding: 0; }}
body {{
  font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, monospace;
  background: var(--base);
  color: var(--text);
  max-width: 1200px;
  margin: 0 auto;
  padding: 24px;
  line-height: 1.5;
}}
h1 {{ color: var(--lavender); font-size: 1.5rem; margin-bottom: 4px; }}
h2 {{ color: var(--subtext1); font-size: 1.1rem; margin: 24px 0 12px; }}
.subtitle {{ color: var(--overlay0); font-size: 0.85rem; margin-bottom: 20px; }}

/* Tab nav */
.tab-nav {{ display: flex; gap: 4px; margin-bottom: 0; }}
.tab-btn {{
  background: var(--surface0);
  color: var(--subtext0);
  border: none;
  padding: 8px 20px;
  cursor: pointer;
  font-size: 0.9rem;
  border-radius: 6px 6px 0 0;
  transition: background 0.2s;
}}
.tab-btn:hover {{ background: var(--surface1); }}
.tab-btn.active {{ background: var(--crust); color: var(--text); }}
.tab-content {{ display: none; background: var(--crust); padding: 20px; border-radius: 0 6px 6px 6px; }}
.tab-content.active {{ display: block; }}

/* Tree */
.tree-node {{ margin-left: 0; }}
.tree-children {{ margin-left: 24px; border-left: 1px solid var(--surface1); padding-left: 8px; }}
.tree-row {{ display: flex; align-items: center; gap: 6px; padding: 3px 0; font-size: 0.9rem; }}
.tree-row summary {{ cursor: pointer; list-style: none; display: flex; align-items: center; gap: 6px; }}
.tree-row summary::-webkit-details-marker {{ display: none; }}
.tree-toggle {{ color: var(--overlay0); font-size: 0.75rem; width: 14px; text-align: center; }}
.status-icon {{ font-size: 0.85rem; }}
.node-label {{ font-weight: 500; }}
.node-meta {{ color: var(--overlay0); font-size: 0.8rem; }}
.duration {{ color: var(--teal); }}
.tokens {{ color: var(--mauve); }}
.rounds {{ color: var(--blue); }}
.error {{ color: var(--red); font-size: 0.8rem; margin-left: 20px; }}

/* Health cards */
.health-grid {{ display: grid; grid-template-columns: repeat(auto-fit, minmax(180px, 1fr)); gap: 12px; margin-bottom: 24px; }}
.health-card {{
  background: var(--mantle);
  border: 1px solid var(--surface1);
  border-radius: 8px;
  padding: 14px;
  text-align: center;
}}
.health-card .value {{ font-size: 1.8rem; font-weight: 700; }}
.health-card .label {{ color: var(--subtext0); font-size: 0.8rem; margin-top: 2px; }}
.green {{ color: var(--green); }}
.red {{ color: var(--red); }}
.yellow {{ color: var(--yellow); }}
.blue {{ color: var(--blue); }}

/* Progress bar */
.progress-bar {{
  background: var(--surface0);
  border-radius: 4px;
  height: 8px;
  margin: 8px 0;
  overflow: hidden;
}}
.progress-fill {{
  height: 100%;
  border-radius: 4px;
  transition: width 0.4s;
}}

/* Failure modes */
.failure-table {{ width: 100%; border-collapse: collapse; margin-top: 12px; }}
.failure-table th, .failure-table td {{ text-align: left; padding: 8px 12px; border-bottom: 1px solid var(--surface1); font-size: 0.85rem; }}
.failure-table th {{ color: var(--subtext0); font-weight: 600; }}
.severity-badge {{
  display: inline-block;
  padding: 2px 8px;
  border-radius: 4px;
  font-size: 0.75rem;
  font-weight: 600;
}}
.severity-Critical {{ background: rgba(243,139,168,0.2); color: var(--red); }}
.severity-Warning {{ background: rgba(249,226,175,0.2); color: var(--yellow); }}
.severity-Info {{ background: rgba(137,180,250,0.2); color: var(--blue); }}

/* Error timeline */
.error-item {{
  background: var(--mantle);
  border-left: 3px solid var(--red);
  padding: 10px 14px;
  margin-bottom: 8px;
  border-radius: 0 6px 6px 0;
}}
.error-item .time {{ color: var(--overlay0); font-size: 0.8rem; }}
.error-item .agent {{ font-weight: 600; }}

/* Toolbar */
.toolbar {{ display: flex; gap: 8px; margin-bottom: 16px; }}
.toolbar button {{
  background: var(--surface0);
  color: var(--subtext0);
  border: 1px solid var(--surface1);
  padding: 4px 12px;
  border-radius: 4px;
  cursor: pointer;
  font-size: 0.8rem;
}}
.toolbar button:hover {{ background: var(--surface1); color: var(--text); }}
.toolbar input {{
  background: var(--surface0);
  color: var(--text);
  border: 1px solid var(--surface1);
  padding: 4px 12px;
  border-radius: 4px;
  font-size: 0.8rem;
  flex: 1;
  max-width: 200px;
}}

/* Responsive */
@media (max-width: 768px) {{
  body {{ padding: 12px; }}
  .health-grid {{ grid-template-columns: repeat(2, 1fr); }}
}}
</style>
</head>
<body>

<h1>🌳 Subagent Trace Report</h1>
<div class="subtitle">Session: <code>{session_id}</code></div>

<div class="tab-nav">
  <button class="tab-btn active" onclick="switchTab('tree')">📊 Call Tree</button>
  <button class="tab-btn" onclick="switchTab('health')">💚 Health Dashboard</button>
  <button class="tab-btn" onclick="switchTab('errors')">⚠ Error Timeline</button>
</div>

<div id="tab-tree" class="tab-content active">
  <div class="toolbar">
    <button onclick="expandAll()">+ Expand All</button>
    <button onclick="collapseAll()">− Collapse All</button>
    <input type="text" id="filterInput" placeholder="Filter nodes…" oninput="filterTree(this.value)">
  </div>
  <div id="tree-root"></div>
</div>

<div id="tab-health" class="tab-content">
  <div id="health-root"></div>
</div>

<div id="tab-errors" class="tab-content">
  <div id="errors-root"></div>
</div>

<script>
const DATA = {{ tree: {tree_str}, health: {health_str} }};
const SESSION_ID = "{session_id}";

// ── Tab switching ─────────────────────────────────────────────────────
function switchTab(name) {{
  document.querySelectorAll('.tab-btn').forEach(b => b.classList.remove('active'));
  document.querySelectorAll('.tab-content').forEach(c => c.classList.remove('active'));
  document.querySelector('.tab-btn[onclick*="' + name + '"]').classList.add('active');
  document.getElementById('tab-' + name).classList.add('active');
}}

// ── Duration formatting ───────────────────────────────────────────────
function fmtDuration(ms) {{
  if (ms >= 60000) return (ms / 60000).toFixed(1) + 'm';
  if (ms >= 1000) return (ms / 1000).toFixed(1) + 's';
  return ms + 'ms';
}}

function fmtTokens(n) {{
  if (n >= 1000) return (n / 1000).toFixed(1) + 'k';
  return n.toString();
}}

// ── Status helpers ────────────────────────────────────────────────────
const STATUS_ICON = {{
  completed: '✅', failed: '❌', cancelled: '🚫', pending: '⏳', running: '🔄'
}};

function statusIcon(s) {{ return STATUS_ICON[s] || '❓'; }}

// ── Tree renderer ─────────────────────────────────────────────────────
function renderTree(nodes, depth, expandedSet) {{
  if (!nodes || nodes.length === 0) return '';
  let html = '';
  for (const node of nodes) {{
    const hasChildren = node.children && node.children.length > 0;
    const isExpanded = depth < 3 || expandedSet.has(node.id);
    const dur = fmtDuration(node.duration_ms || 0);
    const tokens = node.total_tokens ? fmtTokens(node.total_tokens) : null;
    const rounds = node.actual_rounds ? node.actual_rounds + 'r' : null;
    const meta = [rounds, dur, tokens ? tokens + ' tokens' : null].filter(Boolean).join(' · ');

    html += '<div class="tree-node">';
    if (hasChildren) {{
      html += '<details ' + (isExpanded ? 'open' : '') + '>';
      html += '<summary class="tree-row">';
      html += '<span class="tree-toggle">▶</span>';
      html += '<span class="status-icon">' + statusIcon(node.status) + '</span>';
      html += '<span class="node-label">' + escapeHtml(node.label) + '</span>';
      html += '<span class="node-meta">' + meta + '</span>';
      html += '</summary>';
      html += '<div class="tree-children">' + renderTree(node.children, depth + 1, expandedSet) + '</div>';
      html += '</details>';
    }} else {{
      html += '<div class="tree-row">';
      html += '<span class="tree-toggle" style="visibility:hidden">▶</span>';
      html += '<span class="status-icon">' + statusIcon(node.status) + '</span>';
      html += '<span class="node-label">' + escapeHtml(node.label) + '</span>';
      html += '<span class="node-meta">' + meta + '</span>';
      html += '</div>';
    }}

    if (node.error_message) {{
      html += '<div class="error">⚠ ' + escapeHtml(node.error_message) + '</div>';
    }}
    html += '</div>';
  }}
  return html;
}}

function escapeHtml(s) {{
  const d = document.createElement('div');
  d.textContent = s;
  return d.innerHTML;
}}

// ── Health dashboard renderer ─────────────────────────────────────────
function renderHealthDashboard(h) {{
  if (!h || !h.total_runs) {{
    document.getElementById('health-root').innerHTML = '<p style="color:var(--subtext0)">No health data available.</p>';
    return;
  }}
  const scoreColor = h.health_score >= 90 ? 'green' : h.health_score >= 70 ? 'yellow' : h.health_score >= 50 ? 'yellow' : 'red';
  let html = '<div class="health-grid">';
  html += card('Health Score', h.health_score || 0, scoreColor);
  html += card('Success Rate', (h.success_rate * 100).toFixed(1) + '%', h.success_rate >= 0.9 ? 'green' : 'yellow');
  html += card('Total Runs', h.total_runs, 'blue');
  html += card('Completed', h.completed, 'green');
  html += card('Failed', h.failed, 'red');
  html += card('Cancelled', h.cancelled, 'yellow');
  html += card('Avg Rounds', (h.avg_rounds || 0).toFixed(1), 'lavender');
  html += card('Avg Tokens', fmtTokens(Math.round(h.avg_tokens || 0)), 'mauve');
  html += card('Avg Duration', fmtDuration(Math.round(h.avg_duration_ms || 0)), 'teal');
  html += '</div>';

  if (h.status) {{
    html += '<h2>Status: <span class="' + scoreColor + '">' + h.status + '</span></h2>';
  }}

  if (h.failure_modes && h.failure_modes.length > 0) {{
    html += '<h2>Failure Mode Breakdown</h2>';
    html += '<table class="failure-table"><tr><th>Mode</th><th>Count</th><th>%</th><th>Severity</th><th>Recommendation</th></tr>';
    for (const fm of h.failure_modes) {{
      html += '<tr>';
      html += '<td>' + escapeHtml(fm.label) + '</td>';
      html += '<td>' + fm.count + '</td>';
      html += '<td>' + fm.percentage.toFixed(1) + '%</td>';
      html += '<td><span class="severity-badge severity-' + fm.severity + '">' + fm.severity + '</span></td>';
      html += '<td style="color:var(--subtext0)">' + escapeHtml(fm.recommendation || '') + '</td>';
      html += '</tr>';
    }}
    html += '</table>';
  }}

  if (h.recommendations && h.recommendations.length > 0) {{
    html += '<h2>Recommendations</h2><ul style="color:var(--subtext0);padding-left:20px">';
    for (const r of h.recommendations) {{
      html += '<li>' + escapeHtml(r) + '</li>';
    }}
    html += '</ul>';
  }}
  document.getElementById('health-root').innerHTML = html;
}}

function card(label, value, colorClass) {{
  return '<div class="health-card"><div class="value ' + colorClass + '">' + value + '</div><div class="label">' + label + '</div></div>';
}}

// ── Error timeline renderer ───────────────────────────────────────────
function renderErrorTimeline(h) {{
  if (!h || (!h.failed && !h.failure_modes)) {{
    document.getElementById('errors-root').innerHTML = '<p style="color:var(--green)">🎉 No errors recorded.</p>';
    return;
  }}
  let html = '';
  if (h.failure_modes && h.failure_modes.length > 0) {{
    html += '<h2>Failure Modes</h2>';
    for (const fm of h.failure_modes) {{
      html += '<div class="error-item">';
      html += '<div class="agent"><span class="severity-badge severity-' + fm.severity + '">' + fm.severity + '</span> ' + escapeHtml(fm.label) + '</div>';
      html += '<div style="margin-top:4px;color:var(--subtext0)">Count: ' + fm.count + ' (' + fm.percentage.toFixed(1) + '%)</div>';
      if (fm.recommendation) {{
        html += '<div style="margin-top:4px;color:var(--teal)">→ ' + escapeHtml(fm.recommendation) + '</div>';
      }}
      html += '</div>';
    }}
  }}
  document.getElementById('errors-root').innerHTML = html;
}}

// ── Expand/Collapse all ───────────────────────────────────────────────
function expandAll() {{
  document.querySelectorAll('#tree-root details').forEach(d => d.open = true);
}}
function collapseAll() {{
  document.querySelectorAll('#tree-root details').forEach(d => d.open = false);
}}

// ── Filter ────────────────────────────────────────────────────────────
function filterTree(query) {{
  const q = query.toLowerCase();
  document.querySelectorAll('#tree-root .tree-node').forEach(node => {{
    const text = node.textContent.toLowerCase();
    node.style.display = q ? (text.includes(q) ? '' : 'none') : '';
  }});
  if (q) expandAll();
}}

// ── Init ──────────────────────────────────────────────────────────────
document.addEventListener('DOMContentLoaded', function() {{
  document.getElementById('tree-root').innerHTML = renderTree(DATA.tree, 0, new Set());
  renderHealthDashboard(DATA.health);
  renderErrorTimeline(DATA.health);
}});
</script>

</body>
</html>"##
    )
}

// ─── Helper: attach children recursively ───────────────────────────────

fn attach_children(
    nodes: &mut [TraceNode],
    children_map: &HashMap<String, Vec<usize>>,
    parent_idx: usize,
    processed: &mut [bool],
) {
    let parent_id = nodes[parent_idx].id.clone();
    if let Some(child_indices) = children_map.get(&parent_id) {
        let mut children: Vec<TraceNode> = Vec::new();
        for &child_idx in child_indices {
            if !processed[child_idx] {
                processed[child_idx] = true;
                attach_children(nodes, children_map, child_idx, processed);
                children.push(nodes[child_idx].clone());
            }
        }
        children.sort_by_key(|c| c.started_at);
        nodes[parent_idx].children = children;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── safe_truncate ──────────────────────────────────────────────────

    #[test]
    fn test_safe_truncate_ascii_short() {
        assert_eq!(safe_truncate("hello", 10), "hello");
    }

    #[test]
    fn test_safe_truncate_ascii_long() {
        let s = "abcdefghijklmnopqrstuvwxyz";
        let result = safe_truncate(s, 10);
        assert!(result.len() <= 13); // "abcdefgh…"
        assert!(result.ends_with('…'));
    }

    #[test]
    fn test_safe_truncate_unicode_boundary() {
        // '─' is 3 bytes in UTF-8: E2 94 80
        // Build a string where byte-index 4 falls inside '─'
        let s = "ab─defghijkl";
        // bytes: a(0) b(1) ─(2-4) d(5) e(6) ...
        let result = safe_truncate(s, 4);
        // Should not panic; should truncate at char boundary (byte 2, after 'b')
        assert!(result.ends_with('…'));
        assert!(!result.contains('─')); // truncated before the multi-byte char
    }

    #[test]
    fn test_safe_truncate_empty() {
        assert_eq!(safe_truncate("", 10), "");
    }

    #[test]
    fn test_safe_truncate_exact_boundary() {
        assert_eq!(safe_truncate("abcd", 4), "abcd");
    }

    // ── nodes_to_json ──────────────────────────────────────────────────

    #[test]
    fn test_nodes_to_json_empty() {
        let result = nodes_to_json(&[]);
        assert_eq!(result, serde_json::Value::Array(vec![]));
    }

    #[test]
    fn test_nodes_to_json_single() {
        let node = TraceNode {
            id: "n1".into(),
            label: "test-agent".into(),
            status: "completed".into(),
            started_at: 1000,
            finished_at: Some(5000),
            error_message: None,
            total_tokens: 1500,
            actual_rounds: 3,
            summary: Some("done".into()),
            events: vec![TraceEvent {
                round: 1,
                event_type: "action".into(),
                tool_name: Some("read".into()),
                tool_params: Some("src/main.rs".into()),
                elapsed_ms: 200,
                data: "ok".into(),
            }],
            children: vec![],
        };
        let result = nodes_to_json(&[node]);
        let arr = result.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        let n = &arr[0];
        assert_eq!(n["id"], "n1");
        assert_eq!(n["label"], "test-agent");
        assert_eq!(n["status"], "completed");
        assert_eq!(n["duration_ms"], 4000);
        assert_eq!(n["total_tokens"], 1500);
        assert_eq!(n["actual_rounds"], 3);
        assert_eq!(n["is_success"], true);
        assert_eq!(n["children"].as_array().unwrap().len(), 0);
        assert_eq!(n["events"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn test_nodes_to_json_nested() {
        let child = TraceNode {
            id: "c1".into(),
            label: "child".into(),
            status: "completed".into(),
            started_at: 2000,
            finished_at: Some(3000),
            error_message: None,
            total_tokens: 500,
            actual_rounds: 2,
            summary: None,
            events: vec![],
            children: vec![],
        };
        let parent = TraceNode {
            id: "p1".into(),
            label: "parent".into(),
            status: "completed".into(),
            started_at: 1000,
            finished_at: Some(4000),
            error_message: None,
            total_tokens: 1000,
            actual_rounds: 3,
            summary: None,
            events: vec![],
            children: vec![child],
        };
        let result = nodes_to_json(&[parent]);
        let arr = result.as_array().unwrap();
        let p = &arr[0];
        let children = p["children"].as_array().unwrap();
        assert_eq!(children.len(), 1);
        assert_eq!(children[0]["id"], "c1");
        assert_eq!(children[0]["label"], "child");
    }

    #[test]
    fn test_nodes_to_json_unicode() {
        // Node with multi-byte UTF-8 in label
        let node = TraceNode {
            id: "u1".into(),
            label: "─── HTML Report ───".into(),
            status: "completed".into(),
            started_at: 0,
            finished_at: Some(100),
            error_message: None,
            total_tokens: 0,
            actual_rounds: 0,
            summary: None,
            events: vec![],
            children: vec![],
        };
        let result = nodes_to_json(&[node]);
        let s = serde_json::to_string(&result).unwrap();
        assert!(s.contains("─── HTML Report ───"));
    }

    // ── build_html_report ──────────────────────────────────────────────

    #[test]
    fn test_build_html_report_basic() {
        let tree = nodes_to_json(&[]);
        let health = serde_json::json!({
            "period": "All Time",
            "total_runs": 10,
            "completed": 8,
            "failed": 1,
            "cancelled": 1,
            "success_rate": 0.8,
            "avg_rounds": 3.5,
            "avg_tokens": 5000.0,
            "avg_duration_ms": 12000.0,
            "health_score": 80.0,
            "status": "Degraded",
            "failure_modes": [],
            "recommendations": []
        });
        let html = build_html_report(&tree, &health, "test-session");

        // Must contain key structural elements
        assert!(html.contains("<!DOCTYPE html>"));
        assert!(html.contains("<style>"));
        assert!(html.contains("</style>"));
        assert!(html.contains("<script>"));
        assert!(html.contains("</script>"));
        assert!(html.contains("const DATA"));
        assert!(html.contains("tab-tree"));
        assert!(html.contains("tab-health"));
        assert!(html.contains("tab-errors"));
        assert!(html.contains("test-session"));
        assert!(html.contains("renderTree"));
        assert!(html.contains("renderHealthDashboard"));
        assert!(html.contains("renderErrorTimeline"));
    }

    #[test]
    fn test_build_html_report_empty() {
        let tree = nodes_to_json(&[]);
        let health = serde_json::json!({});
        let html = build_html_report(&tree, &health, "empty");
        assert!(html.contains("<!DOCTYPE html>"));
        assert!(!html.is_empty());
    }

    #[test]
    fn test_build_html_report_health_data_embedded() {
        let tree = nodes_to_json(&[]);
        let health = serde_json::json!({
            "total_runs": 5,
            "completed": 5,
            "failed": 0,
            "cancelled": 0,
            "success_rate": 1.0,
            "health_score": 95.0,
            "status": "🟢 Healthy"
        });
        let html = build_html_report(&tree, &health, "s");
        assert!(html.contains("total_runs"));
        assert!(html.contains("95"));
    }
}
