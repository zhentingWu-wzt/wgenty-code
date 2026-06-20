//! Subagent Health Dashboard — observability and analytics for subagent execution.
//!
//! Queries the SubagentTranscriptStore to compute:
//!   - Overall success/failure/cancellation rates
//!   - Per-failure-mode breakdown (timeout, budget, stuck, parse_error, max_rounds, api_error)
//!   - Average rounds/tokens/latency per subagent
//!   - Health score (0–100) with trend analysis
//!   - Recommendation engine based on failure patterns

use crate::transcript::{SubagentTranscriptHeader, SubagentTranscriptStore};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy)]
pub enum HealthPeriod {
    Last1h,
    Last24h,
    Last7d,
    Last30d,
    AllTime,
}

impl HealthPeriod {
    fn cutoff_ms(&self) -> Option<i64> {
        let now = chrono::Utc::now().timestamp_millis();
        match self {
            Self::Last1h => Some(now - 3600 * 1000),
            Self::Last24h => Some(now - 86400 * 1000),
            Self::Last7d => Some(now - 7 * 86400 * 1000),
            Self::Last30d => Some(now - 30 * 86400 * 1000),
            Self::AllTime => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum FailureMode {
    Timeout,
    TokenBudgetExceeded,
    StuckLoop,
    ParseError,
    MaxRoundsExceeded,
    ApiError,
    ToolError,
    Cancelled,
    Unknown,
}

impl FailureMode {
    pub fn classify(error_msg: &str) -> Self {
        let lower = error_msg.to_lowercase();
        if lower.contains("timeout") || lower.contains("timed out") {
            Self::Timeout
        } else if lower.contains("budget") || lower.contains("token") {
            Self::TokenBudgetExceeded
        } else if lower.contains("stuck") || lower.contains("loop") || lower.contains("repeated") {
            Self::StuckLoop
        } else if lower.contains("parse") || lower.contains("json") || lower.contains("malformed") {
            Self::ParseError
        } else if lower.contains("max") && lower.contains("round") {
            Self::MaxRoundsExceeded
        } else if lower.contains("api") || lower.contains("connection") || lower.contains("network") {
            Self::ApiError
        } else if lower.contains("tool") || lower.contains("execut") {
            Self::ToolError
        } else if lower.contains("cancel") {
            Self::Cancelled
        } else {
            Self::Unknown
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Timeout => "Timeout",
            Self::TokenBudgetExceeded => "Token Budget Exceeded",
            Self::StuckLoop => "Stuck Loop Detected",
            Self::ParseError => "Parse Error Cascade",
            Self::MaxRoundsExceeded => "Max Rounds Exceeded",
            Self::ApiError => "API/Network Error",
            Self::ToolError => "Tool Execution Error",
            Self::Cancelled => "Cancelled",
            Self::Unknown => "Unknown",
        }
    }

    pub fn severity(&self) -> &'static str {
        match self {
            Self::Timeout | Self::ApiError => "Critical",
            Self::TokenBudgetExceeded | Self::MaxRoundsExceeded => "Warning",
            Self::StuckLoop | Self::ParseError | Self::ToolError => "Warning",
            Self::Cancelled | Self::Unknown => "Info",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailureModeStats {
    pub mode: FailureMode,
    pub label: String,
    pub count: usize,
    pub percentage: f64,
    pub severity: String,
    pub recommendation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentHealth {
    pub period: String,
    pub total_runs: usize,
    pub completed: usize,
    pub failed: usize,
    pub cancelled: usize,
    pub success_rate: f64,
    pub failure_rate: f64,
    pub avg_rounds: f64,
    pub avg_tokens: f64,
    pub avg_duration_ms: f64,
    pub health_score: f64,
    pub status: HealthStatus,
    pub failure_modes: Vec<FailureModeStats>,
    pub recommendations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum HealthStatus {
    Healthy,
    Degraded,
    Unhealthy,
    Critical,
}

impl HealthStatus {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Healthy => "🟢 Healthy",
            Self::Degraded => "🟡 Degraded",
            Self::Unhealthy => "🟠 Unhealthy",
            Self::Critical => "🔴 Critical",
        }
    }

    pub fn from_success_rate(rate: f64) -> Self {
        if rate >= 0.90 {
            Self::Healthy
        } else if rate >= 0.70 {
            Self::Degraded
        } else if rate >= 0.50 {
            Self::Unhealthy
        } else {
            Self::Critical
        }
    }
}

pub struct SubagentHealthAnalyzer {
    store: std::sync::Arc<SubagentTranscriptStore>,
}

impl SubagentHealthAnalyzer {
    pub fn new(store: std::sync::Arc<SubagentTranscriptStore>) -> Self {
        Self { store }
    }

    pub fn compute_health(
        &self,
        session_id: Option<&str>,
        period: HealthPeriod,
    ) -> Result<SubagentHealth, String> {
        let cutoff = period.cutoff_ms();
        let mut transcripts: Vec<SubagentTranscriptHeader> = if let Some(sid) = session_id {
            self.store
                .list_by_session(sid)
                .map_err(|e| format!("Failed to list transcripts: {}", e))?
        } else {
            let mut all = Vec::new();
            for prefix in &["task:", "subagent:", "sub:", "explore:", "plan:"] {
                if let Ok(results) = self.store.search(prefix) {
                    all.extend(results);
                }
            }
            for sid in &["default", "main"] {
                if let Ok(results) = self.store.list_by_session(sid) {
                    all.extend(results);
                }
            }
            all
        };

        let filtered: Vec<&SubagentTranscriptHeader> = if let Some(cutoff_ms) = cutoff {
            transcripts.iter().filter(|t| t.started_at >= cutoff_ms).collect()
        } else {
            transcripts.iter().collect()
        };

        self.compute_from_headers(&filtered, period)
    }

    pub fn compute_from_headers(
        &self,
        transcripts: &[&SubagentTranscriptHeader],
        period: HealthPeriod,
    ) -> Result<SubagentHealth, String> {
        let total = transcripts.len();
        if total == 0 {
            return Ok(SubagentHealth {
                period: format!("{:?}", period),
                total_runs: 0,
                completed: 0,
                failed: 0,
                cancelled: 0,
                success_rate: 0.0,
                failure_rate: 0.0,
                avg_rounds: 0.0,
                avg_tokens: 0.0,
                avg_duration_ms: 0.0,
                health_score: 0.0,
                status: HealthStatus::Healthy,
                failure_modes: Vec::new(),
                recommendations: vec!["No subagent data yet. Run some subagent tasks to populate the dashboard.".to_string()],
            });
        }

        let completed = transcripts.iter().filter(|t| t.status == "completed").count();
        let failed = transcripts.iter().filter(|t| t.status == "failed").count();
        let cancelled = transcripts.iter().filter(|t| t.status == "cancelled").count();
        let success_rate = completed as f64 / total.max(1) as f64;
        let failure_rate = (failed + cancelled) as f64 / total.max(1) as f64;
        let avg_rounds = transcripts.iter().map(|t| t.actual_rounds as f64).sum::<f64>() / total.max(1) as f64;
        let avg_tokens = transcripts.iter().map(|t| t.total_tokens as f64).sum::<f64>() / total.max(1) as f64;
        let avg_duration_ms = transcripts.iter()
            .filter_map(|t| t.finished_at.map(|f| (f - t.started_at) as f64))
            .sum::<f64>() / total.max(1) as f64;

        let mut mode_counts: HashMap<FailureMode, usize> = HashMap::new();
        for t in transcripts.iter() {
            if t.status == "failed" || t.status == "cancelled" {
                let mode = if t.status == "cancelled" {
                    FailureMode::Cancelled
                } else if let Some(ref err_msg) = t.error_message {
                    FailureMode::classify(err_msg)
                } else {
                    FailureMode::Unknown
                };
                *mode_counts.entry(mode).or_insert(0) += 1;
            }
        }

        let total_failures: usize = mode_counts.values().sum();
        let mut failure_modes: Vec<FailureModeStats> = mode_counts
            .into_iter()
            .map(|(mode, count)| {
                let percentage = if total_failures > 0 { count as f64 / total_failures as f64 * 100.0 } else { 0.0 };
                FailureModeStats {
                    recommendation: recommend(&mode, count, avg_rounds),
                    label: mode.label().to_string(),
                    severity: mode.severity().to_string(),
                    mode, count, percentage,
                }
            })
            .collect();
        failure_modes.sort_by(|a, b| b.count.cmp(&a.count));

        let health_score = score(success_rate, avg_rounds, avg_tokens, &failure_modes);
        let status = HealthStatus::from_success_rate(success_rate);
        let mut recommendations = gen_recs(success_rate, avg_rounds, avg_tokens, &failure_modes, total);
        recommendations.push(format!("Total {} subagent runs analyzed across period {:?}", total, period));

        Ok(SubagentHealth {
            period: format!("{:?}", period), total_runs: total, completed, failed, cancelled,
            success_rate, failure_rate, avg_rounds, avg_tokens, avg_duration_ms,
            health_score, status, failure_modes, recommendations,
        })
    }

    pub fn print_health_report(health: &SubagentHealth) {
        println!();
        println!("{}", "═".repeat(70));
        println!("📊 SUBAGENT HEALTH DASHBOARD — {}  {}", health.status.label(), health.period);
        println!("{}", "═".repeat(70));
        println!();
        println!("┌─ Overview ─────────────────────────────────────────────");
        println!("│ Total runs:        {:>6}", health.total_runs);
        println!("│ ✅ Completed:       {:>6}  ({:.1}%)", health.completed, health.success_rate * 100.0);
        println!("│ ❌ Failed:          {:>6}  ({:.1}%)", health.failed, (health.failed as f64 / health.total_runs.max(1) as f64) * 100.0);
        println!("│ 🚫 Cancelled:       {:>6}  ({:.1}%)", health.cancelled, (health.cancelled as f64 / health.total_runs.max(1) as f64) * 100.0);
        println!("│ 📈 Health Score:    {:>6.0}/100", health.health_score);
        println!("│ 📐 Avg rounds:      {:>6.1}", health.avg_rounds);
        println!("│ 🪙 Avg tokens:      {:>6.0}", health.avg_tokens);
        println!("│ ⏱️  Avg duration:    {:>6.0}ms", health.avg_duration_ms);
        println!("└────────────────────────────────────────────────────────");

        if !health.failure_modes.is_empty() {
            println!();
            println!("┌─ Failure Mode Breakdown ──────────────────────────────");
            for fm in &health.failure_modes {
                let bar = "█".repeat((fm.percentage / 2.0) as usize);
                println!("│ [{}] {:30} {:>4} ({:5.1}%) {}", fm.severity.chars().next().unwrap_or('?'), fm.label, fm.count, fm.percentage, bar);
                println!("│      → {}", fm.recommendation);
            }
            println!("└────────────────────────────────────────────────────────");
        }

        if !health.recommendations.is_empty() {
            println!();
            println!("┌─ Recommendations ─────────────────────────────────────");
            for (i, rec) in health.recommendations.iter().enumerate() {
                println!("│ {}. {}", i + 1, rec);
            }
            println!("└────────────────────────────────────────────────────────");
        }
        println!();
    }
}

fn score(success_rate: f64, avg_rounds: f64, avg_tokens: f64, failure_modes: &[FailureModeStats]) -> f64 {
    let base = success_rate * 70.0;
    let rounds_score = if avg_rounds > 25.0 { 0.0 } else if avg_rounds > 15.0 { 5.0 } else if avg_rounds > 8.0 { 10.0 } else { 15.0 };
    let token_score = if avg_tokens > 50_000.0 { 0.0 } else if avg_tokens > 20_000.0 { 3.0 } else if avg_tokens > 8_000.0 { 7.0 } else { 10.0 };
    let critical: f64 = failure_modes.iter().filter(|fm| fm.severity == "Critical").map(|fm| fm.count as f64).sum();
    let total_f: f64 = failure_modes.iter().map(|fm| fm.count as f64).sum();
    let penalty = if total_f > 0.0 { (critical / total_f) * 15.0 } else { 0.0 };
    (base + rounds_score + token_score - penalty).clamp(0.0, 100.0)
}

fn recommend(mode: &FailureMode, count: usize, _avg_rounds: f64) -> String {
    match mode {
        FailureMode::Timeout => if count >= 3 {
            "Consider increasing subagent.timeout_secs (default 240s) to 480s+".into()
        } else { "Occasional timeouts — monitor API latency".into() },
        FailureMode::TokenBudgetExceeded => "Increase token_budget_k in settings".into(),
        FailureMode::StuckLoop => format!("Stuck detection in {} runs — consider more specific prompts", count),
        FailureMode::ParseError => "JSON parse errors — consider using a more capable model".into(),
        FailureMode::MaxRoundsExceeded => "Subagents hitting max rounds — increase max_rounds or split tasks".into(),
        FailureMode::ApiError => "Check API endpoint availability and rate limits".into(),
        FailureMode::ToolError => "Verify file paths, permissions, and tool availability".into(),
        FailureMode::Cancelled => "User cancelled — not a system issue".into(),
        FailureMode::Unknown => "Unclassified failure — review transcript details".into(),
    }
}

fn gen_recs(success_rate: f64, avg_rounds: f64, avg_tokens: f64, failure_modes: &[FailureModeStats], total: usize) -> Vec<String> {
    let mut r = Vec::new();
    if success_rate < 0.50 { r.push(format!("🔴 CRITICAL: {}% success rate. Immediate investigation required.", (success_rate * 100.0) as u32)); }
    else if success_rate < 0.70 { r.push(format!("🟠 WARNING: {}% success rate. Review failure patterns.", (success_rate * 100.0) as u32)); }
    else if success_rate < 0.90 { r.push(format!("🟡 NOTICE: {}% success rate — monitoring recommended.", (success_rate * 100.0) as u32)); }
    if let Some(top) = failure_modes.first() {
        if top.percentage > 50.0 { r.push(format!("Dominant failure: {} ({}%) — focus here first.", top.label, top.percentage as u32)); }
    }
    if avg_rounds > 20.0 { r.push(format!("High avg rounds ({:.0}/100) — consider splitting complex tasks.", avg_rounds)); }
    if avg_tokens > 40_000.0 && total > 5 { r.push(format!("High avg tokens ({:.0}/run) — if budget is set, raise it.", avg_tokens)); }
    if total < 10 { r.push("Low sample size — more runs needed for accurate metrics.".into()); }
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcript::SubagentTranscriptHeader;

    fn hdr(status: &str, err: Option<&str>, rounds: u32, tokens: u64, started: i64, finished: i64) -> SubagentTranscriptHeader {
        SubagentTranscriptHeader {
            id: uuid::Uuid::new_v4().to_string(), session_id: "test".into(), parent_id: None,
            label: "test".into(), status: status.into(), started_at: started, finished_at: Some(finished),
            total_tokens: tokens, actual_rounds: rounds,
            error_message: err.map(|s| s.into()), summary: None,
        }
    }

    #[test]
    fn test_classify() {
        assert_eq!(FailureMode::classify("Subagent timed out after 600s"), FailureMode::Timeout);
        assert_eq!(FailureMode::classify("Token budget exceeded: limit 50k"), FailureMode::TokenBudgetExceeded);
        assert_eq!(FailureMode::classify("Stuck in loop: repeated 3+ times"), FailureMode::StuckLoop);
        assert_eq!(FailureMode::classify("3 consecutive irrecoverable JSON errors"), FailureMode::ParseError);
        assert_eq!(FailureMode::classify("Subagent exceeded maximum rounds"), FailureMode::MaxRoundsExceeded);
        assert_eq!(FailureMode::classify("API call failed: connection refused"), FailureMode::ApiError);
    }

    #[test]
    fn test_health_perfect() {
        let s = score(1.0, 5.0, 5000.0, &[]);
        assert!(s > 90.0, "perfect should score >90, got {}", s);
    }

    #[test]
    fn test_health_critical() {
        let fm = vec![FailureModeStats { mode: FailureMode::Timeout, label: "T".into(), count: 10, percentage: 80.0, severity: "Critical".into(), recommendation: "".into() }];
        let s = score(0.2, 28.0, 60000.0, &fm);
        assert!(s < 40.0, "critical should score <40, got {}", s);
    }

    #[test]
    fn test_status() {
        assert_eq!(HealthStatus::from_success_rate(0.95), HealthStatus::Healthy);
        assert_eq!(HealthStatus::from_success_rate(0.80), HealthStatus::Degraded);
        assert_eq!(HealthStatus::from_success_rate(0.60), HealthStatus::Unhealthy);
        assert_eq!(HealthStatus::from_success_rate(0.30), HealthStatus::Critical);
    }

    #[test]
    fn test_empty() {
        let a = SubagentHealthAnalyzer { store: std::sync::Arc::new(SubagentTranscriptStore::open(std::path::Path::new("/tmp/h_empty.db")).unwrap()) };
        let h = a.compute_from_headers(&[], HealthPeriod::AllTime).unwrap();
        assert_eq!(h.total_runs, 0);
        assert_eq!(h.status, HealthStatus::Healthy);
    }

    #[test]
    fn test_mixed() {
        let now = chrono::Utc::now().timestamp_millis();
        let hdrs: Vec<SubagentTranscriptHeader> = vec![
            hdr("completed", None, 5, 8000, now - 10000, now),
            hdr("completed", None, 8, 12000, now - 20000, now - 5000),
            hdr("failed", Some("timed out after 600s"), 15, 25000, now - 30000, now - 10000),
            hdr("failed", Some("stuck in loop"), 12, 18000, now - 40000, now - 15000),
            hdr("completed", None, 3, 5000, now - 50000, now - 40000),
        ];
        let a = SubagentHealthAnalyzer { store: std::sync::Arc::new(SubagentTranscriptStore::open(std::path::Path::new("/tmp/h_mixed.db")).unwrap()) };
        let h = a.compute_from_headers(&hdrs.iter().collect::<Vec<_>>(), HealthPeriod::AllTime).unwrap();
        assert_eq!(h.total_runs, 5);
        assert_eq!(h.completed, 3);
        assert_eq!(h.failed, 2);
        assert!((h.success_rate - 0.6).abs() < 0.01);
        assert_eq!(h.failure_modes.len(), 2);
        assert_eq!(h.status, HealthStatus::Unhealthy);
    }
}
