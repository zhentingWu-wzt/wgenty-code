## 1. Failure Diagnostics Data Model & Capture

- [x] 1.1 Extend `ErrorInfo` (`src/agent/progress.rs`) with `root_cause: FailureRootCause`, `failed_tool_sequence: Vec<ToolCallStep>`, `failed_round_context: Option<FailedRoundContext>`, `retry_history: Vec<RetryAttempt>`; keep `retryable: bool` for backward compat; all new fields `#[serde(default)]`
- [x] 1.2 Define `FailureRootCause` enum (TokenBudgetExceeded/GuardianRejected{reason}/SandboxFailed/ApiError/ToolPanic/Timeout/UserCancelled/Unknown) and `ToolCallStep`/`FailedRoundContext`/`RetryAttempt` structs
- [x] 1.3 Extend `FailureMode::classify` (`src/teams/subagent_health.rs`) to emit `FailureRootCause` from structured signals at capture site; add GuardianRejected/SandboxFailed/ToolPanic categories; keep string-match as `Unknown` fallback
- [x] 1.4 In `subagent_loop.rs`, populate `failed_tool_sequence` (from `action_log` failing-round slice, with redacted param summaries + elapsed_ms), `failed_round_context` (assistant text + final tool output, char-boundary truncated to `context_char_limit`), and `root_cause` at failure time
- [x] 1.5 Record `retry_history` per retry attempt (error/root_cause/strategy/outcome) in the retry path; leave empty on no-retry
- [x] 1.6 Add redaction helper for sensitive keys (api_key/token/secret/password) applied to `ToolCallStep` param summaries and trace emission; reuse guardian redaction policy

## 2. Transcript Storage Adaptation

- [ ] 2.1 Add idempotent migration in `run_migrations` (`src/transcript/store.rs`) to `ALTER TABLE subagent_transcripts ADD COLUMN` `failure_diagnostics TEXT`, `root_cause TEXT`, `retry_history TEXT` (guarded by `PRAGMA table_info` presence check)
- [ ] 2.2 Extend `SubagentTranscriptHeader` serialization + `insert`/`get_by_id` to round-trip the new diagnostics columns; map NULL to `Unknown`/empty on read (graceful degradation for old rows)
- [ ] 2.3 Write diagnostics columns in the same transaction as the header row on failure; leave NULL on success
- [ ] 2.4 Add/extend unit tests: empty-db migration, old-db migration (no data loss), NULL-column degradation, diagnostics round-trip

## 3. Trace Streaming (JSONL File + Daemon SSE)

- [ ] 3.1 Create `src/teams/trace_sink.rs` `TraceSink` driven by `ProgressCallback`: append JSONL events to `<subagent.trace.dir>/<session_id>.jsonl` with 0600 file / 0700 dir permissions; apply sensitive-param redaction
- [ ] 3.2 Wire `TraceSink` into the subagent dispatch path so it receives progress events; honor `subagent.trace.sink` (`file`|`daemon`|`both`|`off`, default `file`)
- [ ] 3.3 Add bounded broadcast channel for trace events; on full, drop oldest for live subscribers only (persistence unaffected)
- [ ] 3.4 Add `GET /api/v1/subagents/trace/stream` SSE endpoint (feature-gated `daemon`) with `require_auth`, `session_id` and `since` query params; replay persisted history from transcript store on cold start, then stream live
- [ ] 3.5 Tests: JSONL append + redaction, sink disabled by config, SSE auth rejection, session filter, cold-start replay, backpressure drops oldest (persistence intact)

## 4. CLI Subagent Subcommand

- [ ] 4.1 Add `Commands::Subagent { action: SubagentCommands }` and `SubagentCommands::{List, Trace, Health}` to `src/cli/mod.rs` with clap args (`--session`, `--status`, `--limit`, `--format`, `--raw`, `--period`, `--output`)
- [ ] 4.2 Implement `list`: query transcript store, print reverse-chronological table (id/label/status/root-cause/duration/started_at) with filters
- [ ] 4.3 Implement `trace <id>`: load by id, reuse trace rendering with `--format` (default call_tree) and `--raw` (print diagnostics JSON); non-zero exit on unknown id
- [ ] 4.4 Implement `health`: call `SubagentHealthAnalyzer::compute_from_headers` with `--period`, print total/completed/failed/success-rate + failure-mode breakdown grouped by `FailureRootCause`
- [ ] 4.5 Tests: list filter/sort, trace format variants + unknown id exit code, health period windows + root-cause grouping

## 5. Trace Rendering Adaptation

- [ ] 5.1 Extend `SubagentTraceTool` / trace rendering to surface `root_cause` + `failed_tool_sequence` (with per-step durations) in `call_tree`
- [ ] 5.2 Extend `error_timeline` to group by `FailureRootCause` and include `retry_history`
- [ ] 5.3 Extend `html` report with a failure-diagnostics section (root cause, failed sequence, failed-round context, retry history); keep self-contained, UTF-8 char-boundary safe
- [ ] 5.4 Add raw-mode rendering that prints stored diagnostics as pretty JSON

## 6. Config, Docs & Integration

- [ ] 6.1 Add config keys `subagent.trace.sink`, `subagent.trace.dir`, `subagent.trace.context_char_limit` to settings schema with defaults; document in WGENTY.md config table
- [ ] 6.2 Document `wgenty-code subagent list|trace|health` in WGENTY.md CLI subcommand table
- [ ] 6.3 Run `cargo fmt`, `cargo clippy --all-targets -- -D warnings` (zero warning), `cargo test --all`; fix any regressions
- [ ] 6.4 Verify cross-platform compile (linux/macos/windows) and `daemon` feature gating of SSE endpoint
