# Comet Design Handoff

- Change: subagent-chain-tracing
- Phase: design
- Mode: compact
- Context hash: bbd37f5ea8acf23ccecf9df1e1e58730b4a999212b097bc2c3757e6d7d737e86

Generated-by: comet-handoff.sh

OpenSpec remains the canonical capability spec. This handoff is a deterministic, source-traceable context pack, not an agent-authored summary.

## openspec/changes/subagent-chain-tracing/proposal.md

- Source: openspec/changes/subagent-chain-tracing/proposal.md
- Lines: 1-37
- SHA256: dbda3b43f3526ac232fa37012b60145cf14159fa33c39dac6e79c89e1cd3498f

```md
## Why

Subagent 链路追踪的基础设施（`SubagentProgress` 事件流、`SubagentHealthAnalyzer` 成功率统计、`SubagentTranscriptStore` 持久化、`SubagentTraceTool` 渲染）已存在，但存在三个缺口：

1. **无独立 CLI 查询入口**：现有 `subagent_trace` 工具只能在会话内由 agent 调用且依赖 `session_id`，无法脱离会话查看历史 subagent 运行与成功率。
2. **trace 无法实时流式输出**：只能事后在会话内查询，外部工具（dashboard / Perfetto）无法实时订阅。
3. **失败原因粒度不足**：`ErrorInfo` 仅保留最后一个工具与布尔 `retryable`，无法还原"怎么走到失败"的完整工具调用序列、根因分类、失败轮次上下文与重试历史。

本次补齐这三个缺口，使 subagent 执行可被完整还原、实时观测与离线复盘。

## What Changes

- **失败诊断数据模型**：扩展失败捕获，记录完整失败工具调用序列（tool name + 关键参数 + 每步耗时）、失败根因分类标记（token 超预算 / guardian 拒绝 / sandbox 失败 / API 错误 / 工具 panic / 超时 / 用户取消 / 未知）、失败轮次完整上下文（assistant 文本 + 工具原始输出，截断存储）、重试历史（每次重试错误 + 重试策略 + 最终是否成功）。
- **实时流式 trace 输出**：新增本地 JSONL trace 文件 sink（`<project>/.wgenty-code/traces/<session_id>.jsonl`）与 daemon SSE endpoint（`GET /api/v1/subagents/trace/stream`），外部工具可实时订阅。
- **CLI 历史查询**：新增 `wgenty-code subagent` CLI 子命令（`list` / `trace` / `health`），脱离会话查看历史运行、单次完整 trace、成功率与失败模式统计。
- **存储与渲染适配**：`SubagentTranscriptStore` 持久化新增失败诊断字段；`SubagentTraceTool` 渲染（call_tree / error_timeline / chrome_trace / html）展示新失败详情。

## Capabilities

### New Capabilities

- `subagent-failure-diagnostics`: 失败根因分类、完整失败工具调用序列（含每步耗时）、失败轮次完整上下文（截断）、重试历史的捕获与数据模型契约。
- `subagent-trace-streaming`: subagent 执行 trace 实时流式输出到本地 JSONL 文件 sink 与 daemon SSE endpoint，供外部实时订阅。
- `subagent-cli-tracing`: 独立 CLI 子命令 `wgenty-code subagent list|trace|health`，离线查询历史 subagent 运行、单次 trace、成功率与失败模式。

### Modified Capabilities

- `subagent-transcript-storage`: 持久化 schema 扩展以存储新增失败诊断字段（完整工具调用序列、根因分类、失败轮次上下文、重试历史），并支持 CLI 历史查询与流式 sink 读取。
- `subagent-trace-html-report`: trace 渲染（call_tree / error_timeline / chrome_trace / html）展示新增失败诊断详情。

## Impact

- **代码**：`src/agent/progress.rs`（`SubagentProgress` / `ErrorInfo` 扩展）、`src/teams/subagent_loop.rs`（失败捕获 + 重试历史）、`src/teams/subagent_health.rs`（根因分类统计）、`src/teams/subagent_trace.rs`（渲染新字段）、transcript store（schema 迁移）、`src/cli/`（新增 `Subagent` 子命令）、`src/daemon/`（新增 SSE endpoint）、新增 trace 文件 sink 模块。
- **配置**：新增 `subagent.trace.sink`（`file` | `daemon` | `both` | `off`，默认 `file`）、`subagent.trace.dir`；保留窗口复用 `storage.transcript.max_age_days`。
- **依赖**：daemon feature 已有 axum / tower，SSE 复用现有基础设施；无新外部依赖。
- **存储**：transcript schema 变更需向后兼容迁移（旧记录缺字段降级为 `unknown` / 空）。
- **安全**：trace 文件含工具参数，需遵循现有 guardian 脱敏策略；daemon endpoint 复用现有鉴权。
```

## openspec/changes/subagent-chain-tracing/design.md

- Source: openspec/changes/subagent-chain-tracing/design.md
- Lines: 1-103
- SHA256: c97b298466d0fbea17be554752d51cdd96fc890e0f0734e372b9ca5abb9927b8

[TRUNCATED]

```md
---
comet_change: subagent-chain-tracing
role: technical-design
canonical_spec: openspec
---

## Context

Subagent 链路追踪的基础设施已存在但分散且有缺口：

- `SubagentProgress`（`src/agent/progress.rs`）含 `action_log`/`events`/`error_details: ErrorInfo`（仅 `error_type`/`message`/`last_tool`/`last_params`/`round`/`retryable: bool`）。
- `SubagentTranscriptStore`（`src/transcript/store.rs`）SQLite 持久化两张表：`subagent_transcripts`（header）+ `subagent_events`（每事件含 `tool_name`/`tool_params`/`elapsed_ms`/`round`/`token_count`）。事件级工具调用序列**已存在**，但失败时未沉淀为结构化诊断。
- `FailureMode::classify`（`src/teams/subagent_health.rs`）已做 post-hoc 字符串匹配分类（token/timeout/api/tool/cancel 等），但缺 guardian-reject/sandbox-fail/tool-panic，且结果不落库。
- `SubagentTraceTool`（`src/tools/meta/subagent_trace.rs`）渲染 call_tree/error_timeline/chrome_trace/html，需 `session_id`，仅会话内可用。
- daemon（`src/daemon/routes.rs`）axum + `require_auth`，`chat_stream` 已是流式端点；flat `/api/v1/subagent/progress` 已退役为 capability-scoped 视图。
- CLI（`src/cli/mod.rs`）`Commands` enum 无 `Subagent` 子命令。

约束：MSRV 1.75、CI 零 warning、跨平台、`daemon` feature-gated、transcript schema 须向后兼容。

## Goals / Non-Goals

**Goals:**
- 失败时沉淀结构化诊断：完整失败工具调用序列（含每步耗时）、根因分类、失败轮次完整上下文（截断）、重试历史。
- 实时流式 trace 输出到本地 JSONL 文件 + daemon SSE endpoint。
- 独立 CLI `wgenty-code subagent list|trace|health` 离线查询历史运行、单次 trace、成功率与失败模式。

**Non-Goals:**
- 不改变现有 subagent 调度/重试策略本身（仅记录重试历史，不调整重试逻辑）。
- 不新增对外部可观测性后端（OTel/Jaeger）的导出（未来可扩展）。
- 不做跨项目聚合统计（仅项目本地 transcript）。
- 不重构 capability-scoped agent 路由。

## Decisions

### D1: 失败诊断数据模型--扩展 `ErrorInfo` 而非新表

扩展 `ErrorInfo` 增加：`root_cause: FailureRootCause`（枚举：TokenBudgetExceeded/GuardianRejected{reason}/SandboxFailed/ApiError/ToolPanic/Timeout/UserCancelled/Unknown）、`failed_tool_sequence: Vec<ToolCallStep>`（tool_name + 关键参数摘要 + elapsed_ms，从 `action_log` 失败轮次切片）、`failed_round_context: Option<FailedRoundContext>`（assistant 文本 + 末尾工具原始输出，各截断至 N 字符）、`retry_history: Vec<RetryAttempt>`（每次 attempt 的 error/root_cause/strategy/outcome）。

**为何不新建诊断表**：诊断是 transcript 的派生视图，强生命周期绑定；存为 header 的 JSON 列（`failure_diagnostics TEXT` 存 serde_json）避免多表 join，读取与 header 同步。事件级序列已存在 `subagent_events`，无需重复存储。

**替代方案**：新建 `subagent_failure_diagnostics` 表--被否决，增加 join 复杂度且 1:1 绑定 header。

### D2: 根因分类在捕获时确定，扩展 `FailureMode`

捕获路径（`subagent_loop.rs`）在产生错误时调用扩展后的分类器，写入结构化 `root_cause`。新增类别 GuardianRejected/SandboxFailed/ToolPanic。保留 `FailureMode::classify` 字符串匹配作为旧记录降级路径（`unknown`）。

**替代方案**：完全沿用 post-hoc 字符串匹配--被否决，guardian/sandbox 拒绝信息在错误消息中不稳定，捕获时有结构化信号更可靠。

### D3: 流式 sink--双通道（项目本地 JSONL 文件 + 全局 daemon SSE），异步 buffered writer 驱动

新增 `TraceSink`（`src/teams/trace_sink.rs`）：内部持 mpsc channel，spawn 独立 writer task 批量 flush，**不阻塞** `ProgressCallback`/agent loop（Q3 决策）。复用现有 `ProgressCallback`（`Arc<dyn Fn(SubagentProgress)>`），progress 更新时序列化为 JSONL 事件投递到 channel；writer task append 到**项目本地** `<project>/.wgenty-code/traces/<session_id>.jsonl`（Q2 决策，由运行中 session 写入，0600/0700 权限），并广播到 daemon SSE 订阅者。channel 满时丢最旧事件（仅影响实时订阅，持久在 db 不受影响）。

daemon 端新增 `GET /api/v1/subagents/trace/stream?session_id=&since=`（SSE，复用 `require_auth`），**全局**订阅所有 session（Q5 决策），从内存 broadcast channel 推送；冷启动历史通过全局 `transcript_store` 回放。

**为何双通道**：JSONL 文件为持久离线分析（外部 tail/Perfetto 导入），SSE 为实时订阅。文件 sink 默认开（`subagent.trace.sink=file`），SSE 随 daemon 启用。

**为何异步 buffered**：同步 append 每事件写盘会阻塞 agent loop；mpsc + writer task 解耦，失败不影响主循环。

**替代方案**：仅 daemon SSE--被否决，无 daemon 时丢失持久 trace。仅文件--被否决，无实时订阅。同步 append--被否决，阻塞主循环。

### D4: CLI 子命令复用 transcript store，输出格式化

新增 `Commands::Subagent { action: SubagentCommands }`，`SubagentCommands::{List, Trace, Health}`。`List` 调 `list_by_session`/全量（按时间倒序，表格输出）；`Trace { id }` 调 `get_by_id` + 渲染（复用 `SubagentTraceTool` 渲染逻辑，默认 call_tree）；`Health { --period }` 调 `SubagentHealthAnalyzer::compute_from_headers`。全部只读，直接读 SQLite，无需启动 agent。

**替代方案**：复用现有 `Agent` 子命令--被否决，`Agent` 是"运行 agent"，语义不同。

### D5: schema 迁移--幂等 `ALTER TABLE ADD COLUMN`（4 列）

`run_migrations` 在现有 `execute_batch` 之外增加独立迁移步骤，为 `subagent_transcripts` 加 4 列：`failure_diagnostics TEXT`、`root_cause TEXT`、`retry_history TEXT`、`project_path TEXT`（Q1 决策，写入时记录 project root，CLI v1 暂不暴露项目过滤，为未来预留）。SQLite `ALTER TABLE ADD COLUMN` 不支持 `IF NOT EXISTS`，用 `PRAGMA table_info` 检查列存在性后再执行，保证幂等。旧记录新列 NULL，读取时 `root_cause` 降级为 `Unknown`、其余为空。

**db 位置**（Q1）：transcript db 默认全局 `~/.wgenty-code/subagent_transcripts.db`（`settings.storage.transcript.db_path` 可配）；CLI `list` 全局显示 + `--session`/`--status`/`--limit` 过滤，直接读 SQLite，不启动 agent。

### D6: 配置项

`subagent.trace.sink`（`file`|`daemon`|`both`|`off`，默认 `file`）、`subagent.trace.dir`（默认 `<project>/.wgenty-code/traces`）、`subagent.trace.context_char_limit`（默认 2000，失败轮次上下文截断阈值）。保留窗口复用 `storage.transcript.max_age_days`。

## Risks / Trade-offs

- **trace 文件含工具参数泄露敏感信息** -> 复用 guardian 脱敏策略，`tool_params` 序列化前过滤已知敏感键（api_key/token/secret）；文件权限 0600。
- **SSE 长连接在高并发子代理下内存增长** -> broadcast channel 设容量上限，满则丢弃最旧事件（仅实时订阅受影响，持久记录在文件不受影响）。
```

Full source: openspec/changes/subagent-chain-tracing/design.md

## openspec/changes/subagent-chain-tracing/tasks.md

- Source: openspec/changes/subagent-chain-tracing/tasks.md
- Lines: 1-45
- SHA256: 9cc0b62a3582e311392c466f84d8e8baf40ef63f0c227cac636f7f80e17c29cb

```md
## 1. Failure Diagnostics Data Model & Capture

- [ ] 1.1 Extend `ErrorInfo` (`src/agent/progress.rs`) with `root_cause: FailureRootCause`, `failed_tool_sequence: Vec<ToolCallStep>`, `failed_round_context: Option<FailedRoundContext>`, `retry_history: Vec<RetryAttempt>`; keep `retryable: bool` for backward compat; all new fields `#[serde(default)]`
- [ ] 1.2 Define `FailureRootCause` enum (TokenBudgetExceeded/GuardianRejected{reason}/SandboxFailed/ApiError/ToolPanic/Timeout/UserCancelled/Unknown) and `ToolCallStep`/`FailedRoundContext`/`RetryAttempt` structs
- [ ] 1.3 Extend `FailureMode::classify` (`src/teams/subagent_health.rs`) to emit `FailureRootCause` from structured signals at capture site; add GuardianRejected/SandboxFailed/ToolPanic categories; keep string-match as `Unknown` fallback
- [ ] 1.4 In `subagent_loop.rs`, populate `failed_tool_sequence` (from `action_log` failing-round slice, with redacted param summaries + elapsed_ms), `failed_round_context` (assistant text + final tool output, char-boundary truncated to `context_char_limit`), and `root_cause` at failure time
- [ ] 1.5 Record `retry_history` per retry attempt (error/root_cause/strategy/outcome) in the retry path; leave empty on no-retry
- [ ] 1.6 Add redaction helper for sensitive keys (api_key/token/secret/password) applied to `ToolCallStep` param summaries and trace emission; reuse guardian redaction policy

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
```

## openspec/changes/subagent-chain-tracing/specs/subagent-cli-tracing/spec.md

- Source: openspec/changes/subagent-chain-tracing/specs/subagent-cli-tracing/spec.md
- Lines: 1-45
- SHA256: 4dd94c8fc87b98745a381d59dc0fcebae26a9b4c713a63c59929fb20bda13d46

```md
## ADDED Requirements

### Requirement: Subagent CLI subcommand
The system SHALL provide a `wgenty-code subagent` CLI subcommand with `list`, `trace`, and `health` sub-actions that read directly from the project-local transcript store, without starting an agent loop. All sub-actions SHALL be read-only.

#### Scenario: Subcommand help
- **WHEN** `wgenty-code subagent --help` is run
- **THEN** the `list`, `trace`, and `health` sub-actions SHALL be documented with their options

### Requirement: List historical subagent runs
`wgenty-code subagent list` SHALL list historical subagent runs in reverse chronological order, showing at minimum: transcript id, label, status, root-cause (when failed), duration, and started-at timestamp. It SHALL support optional `--session <id>` (filter by session), `--status <status>` (filter by status), and `--limit <n>` (default 20).

#### Scenario: List recent runs
- **WHEN** `wgenty-code subagent list` is run
- **THEN** the most recent runs SHALL be printed as a table in reverse chronological order, capped at `--limit`

#### Scenario: Filter by status
- **WHEN** `wgenty-code subagent list --status failed` is run
- **THEN** only runs with Failed status SHALL be listed, each annotated with its root cause

### Requirement: Show single subagent trace
`wgenty-code subagent trace <id>` SHALL render the full trace of a single subagent run, reusing the existing trace rendering logic. It SHALL support `--format <call_tree|error_timeline|chrome_trace|html>` (default `call_tree`) and `--raw` (print the raw stored error message and failed-round context without rendering).

#### Scenario: Default call-tree rendering
- **WHEN** `wgenty-code subagent trace <id>` is run
- **THEN** the trace SHALL be rendered as an ASCII call tree including the failed tool-call sequence and root cause when the run failed

#### Scenario: HTML format output
- **WHEN** `wgenty-code subagent trace <id> --format html` is run
- **THEN** a self-contained HTML report SHALL be written to stdout (or `--output <file>`)

#### Scenario: Unknown id
- **WHEN** `wgenty-code subagent trace <unknown-id>` is run
- **THEN** the command SHALL exit non-zero with a clear "not found" error message

### Requirement: Health summary
`wgenty-code subagent health` SHALL print subagent health statistics computed from transcript headers: total runs, completed, failed, success rate, and failure-mode breakdown. It SHALL support `--period <1h|24h|7d|30d|all>` (default `24h`).

#### Scenario: Default 24h health
- **WHEN** `wgenty-code subagent health` is run
- **THEN** the 24-hour window statistics SHALL be printed, including success rate and failure-mode counts

#### Scenario: Failure-mode breakdown with root causes
- **WHEN** `wgenty-code subagent health --period 7d` is run and failures exist
- **THEN** the breakdown SHALL group failures by `FailureRootCause` category with counts
```

## openspec/changes/subagent-chain-tracing/specs/subagent-failure-diagnostics/spec.md

- Source: openspec/changes/subagent-chain-tracing/specs/subagent-failure-diagnostics/spec.md
- Lines: 1-49
- SHA256: 8200a3a30924ed829deacbe3f5415443e2774c71b12566024e9d035f562818fc

```md
## ADDED Requirements

### Requirement: Failure root-cause classification
The system SHALL classify every subagent failure into a structured `FailureRootCause` enum captured at failure time: `TokenBudgetExceeded`, `GuardianRejected` (with reason), `SandboxFailed`, `ApiError`, `ToolPanic`, `Timeout`, `UserCancelled`, or `Unknown`. Classification SHALL be determined from structured signals available at the capture site (e.g., guardian decision, sandbox error variant, token-budget counter), not solely from post-hoc string matching of the error message.

#### Scenario: Guardian rejection classified structurally
- **WHEN** a subagent fails because the guardian denied a tool call
- **THEN** the failure `root_cause` SHALL be `GuardianRejected` carrying the guardian's denial reason, regardless of the error message wording

#### Scenario: Token budget exceeded classified from counter
- **WHEN** a subagent fails because its cumulative token usage exceeds the configured budget
- **THEN** the failure `root_cause` SHALL be `TokenBudgetExceeded`, determined from the token counter rather than message text

#### Scenario: Unknown fallback preserves raw message
- **WHEN** a failure cannot be mapped to a known root cause
- **THEN** the `root_cause` SHALL be `Unknown` and the original `error_message` SHALL be preserved verbatim for manual inspection

### Requirement: Complete failed tool-call sequence captured
On failure, the system SHALL capture the complete ordered sequence of tool calls executed during the failing round (or the failing attempt), each entry recording tool name, a redacted summary of key parameters, and per-step elapsed milliseconds--not only the last tool call.

#### Scenario: Full sequence retained on failure
- **WHEN** a subagent fails after invoking tools A, B, C in the failing round
- **THEN** the failure diagnostics SHALL contain all three tool-call steps (A, B, C) in order, each with its tool name, redacted parameter summary, and elapsed_ms

#### Scenario: Sensitive parameters redacted
- **WHEN** a tool call in the failing sequence carries parameters with sensitive keys (api_key, token, secret, password)
- **THEN** those values SHALL be redacted in the captured parameter summary before persistence or emission

### Requirement: Failed-round full context captured
On failure, the system SHALL capture the failing round's assistant text and the final tool's raw output, each truncated to a configurable character limit (`subagent.trace.context_char_limit`, default 2000), to allow post-hoc reconstruction of the subagent's reasoning at failure.

#### Scenario: Assistant text and tool output retained truncated
- **WHEN** a subagent fails in round N
- **THEN** the diagnostics SHALL include the round-N assistant text and the final tool raw output, each truncated at char boundaries to the configured limit

#### Scenario: Truncation is char-boundary safe
- **WHEN** the configured truncation limit falls within a multi-byte UTF-8 character
- **THEN** truncation SHALL adjust to the nearest valid character boundary without panicking

### Requirement: Retry history recorded
When a subagent execution is retried, the system SHALL record a `RetryAttempt` per attempt, capturing that attempt's error, root cause, the retry strategy that triggered the retry, and the final outcome (succeeded/failed). The historical `retryable: bool` flag SHALL remain available for backward compatibility.

#### Scenario: Multiple retries each recorded
- **WHEN** a subagent is retried twice (attempts 1, 2, 3) and attempt 3 succeeds
- **THEN** the diagnostics SHALL contain three `RetryAttempt` entries with their respective errors/root causes and a final `succeeded` outcome on attempt 3

#### Scenario: No retry yields empty history
- **WHEN** a subagent fails with no retries
- **THEN** `retry_history` SHALL be empty and `retryable` SHALL reflect whether a retry was permitted
```

## openspec/changes/subagent-chain-tracing/specs/subagent-trace-html-report/spec.md

- Source: openspec/changes/subagent-chain-tracing/specs/subagent-trace-html-report/spec.md
- Lines: 1-23
- SHA256: 51e8135d5645d3e4d95e5612f994a3b1918e9870f6819caf978812018a73d377

```md
## ADDED Requirements

### Requirement: Failure diagnostics surfaced in trace rendering
The trace rendering (`call_tree`, `error_timeline`, `chrome_trace`, `html`) SHALL surface the structured failure diagnostics when a subagent failed: the `FailureRootCause` category (and guardian reason when applicable), the complete failed tool-call sequence with per-step elapsed time, the truncated failed-round context (assistant text + final tool output), and the retry history.

#### Scenario: Call tree shows failed sequence and root cause
- **WHEN** a failed subagent trace is rendered with `call_tree`
- **THEN** the output SHALL include the root-cause category and the ordered failed tool-call sequence with per-step durations

#### Scenario: Error timeline groups by root cause
- **WHEN** a failed subagent trace is rendered with `error_timeline`
- **THEN** the breakdown SHALL group failures by `FailureRootCause` category and include retry-history entries

#### Scenario: HTML report includes diagnostics section
- **WHEN** a failed subagent trace is rendered with `html`
- **THEN** the report SHALL include a failure-diagnostics section with root cause, failed sequence, failed-round context, and retry history

### Requirement: Raw diagnostics output
The trace rendering SHALL support a raw mode that prints the stored failure diagnostics JSON (root cause, failed sequence, failed-round context, retry history) without rendering, for piping to external tools.

#### Scenario: Raw mode emits diagnostics JSON
- **WHEN** a failed subagent trace is rendered with raw mode
- **THEN** the stored diagnostics SHALL be printed as pretty JSON to stdout
```

## openspec/changes/subagent-chain-tracing/specs/subagent-trace-streaming/spec.md

- Source: openspec/changes/subagent-chain-tracing/specs/subagent-trace-streaming/spec.md
- Lines: 1-45
- SHA256: fe055328974177246ee4b36f06ee384cbeac8f284abc2b4dd9bdbe15c39d0161

```md
## ADDED Requirements

### Requirement: Local JSONL trace file sink
The system SHALL, when `subagent.trace.sink` is `file` or `both`, append each subagent progress event as a JSONL line to `<subagent.trace.dir>/<session_id>.jsonl` (default `<project>/.wgenty-code/traces/<session_id>.jsonl`). The sink SHALL be driven by the existing `ProgressCallback` and SHALL create the directory with restrictive permissions (0600 file, 0700 dir) on first write.

#### Scenario: Trace events appended as JSONL
- **WHEN** a subagent emits progress events during execution
- **THEN** each event SHALL be serialized as one JSON object per line and appended to the session's trace file

#### Scenario: Sensitive parameters redacted in trace file
- **WHEN** a progress event contains tool parameters with sensitive keys
- **THEN** those values SHALL be redacted before being written to the JSONL file

#### Scenario: Sink disabled by config
- **WHEN** `subagent.trace.sink` is `off`
- **THEN** no trace file SHALL be written and the progress callback SHALL skip the file sink

### Requirement: Daemon SSE trace streaming endpoint
When the `daemon` feature is enabled and the daemon is running, the system SHALL expose `GET /api/v1/subagents/trace/stream` returning a Server-Sent Events stream of subagent trace events, protected by the existing daemon auth middleware. The endpoint SHALL accept optional `session_id` (filter to one session) and `since` (event cursor) query parameters.

#### Scenario: Authenticated live subscription
- **WHEN** an authenticated client connects to the SSE endpoint
- **THEN** subsequent subagent trace events SHALL be pushed to the client in real time as SSE `data:` frames

#### Scenario: Unauthenticated request rejected
- **WHEN** a client connects without valid auth credentials
- **THEN** the endpoint SHALL reject the request with the same auth failure behavior as other protected daemon routes

#### Scenario: Session-filtered stream
- **WHEN** a client connects with `?session_id=<id>`
- **THEN** only trace events for that session SHALL be pushed

### Requirement: Daemon SSE cold-start replay
When a client connects to the SSE endpoint with a `since` cursor or for a known session, the system SHALL replay persisted history from the transcript store before streaming live events, so late subscribers do not lose prior events.

#### Scenario: Late subscriber receives history then live
- **WHEN** a client connects after a subagent has already emitted events
- **THEN** the endpoint SHALL first replay persisted events for the session (or since the cursor) and then continue with live events

### Requirement: Broadcast channel bounded
The in-memory broadcast channel feeding the SSE endpoint SHALL have a bounded capacity; when full, the oldest events SHALL be dropped for live subscribers. Dropped events SHALL remain available via the persisted JSONL file and transcript store, so persistence is not affected by live-subscriber backpressure.

#### Scenario: Backpressure drops oldest for live subscribers only
- **WHEN** the broadcast channel is full and a new event arrives
- **THEN** the oldest buffered event SHALL be dropped from the live channel, but the event SHALL still be persisted to the JSONL file and transcript store
```

## openspec/changes/subagent-chain-tracing/specs/subagent-transcript-storage/spec.md

- Source: openspec/changes/subagent-chain-tracing/specs/subagent-transcript-storage/spec.md
- Lines: 1-37
- SHA256: d0ce7a9238d817723f2c164386cc993bbcd77d031b45a4da013ac807efe40f91

```md
## MODIFIED Requirements

### Requirement: Transcript database schema
The system SHALL maintain a SQLite database at `~/.wgenty-code/subagent_transcripts.db` with tables for transcript headers and per-round events. The `subagent_transcripts` table SHALL additionally include columns `failure_diagnostics` (JSON text, nullable), `root_cause` (text, nullable), and `retry_history` (JSON text, nullable) to persist structured failure diagnostics.

#### Scenario: Database created on first use
- **WHEN** the first subagent transcript is written and the database file does not exist
- **THEN** the system SHALL create the database file with the correct schema (including the new diagnostics columns) automatically

#### Scenario: Transcript header row written on subagent completion
- **WHEN** a subagent reaches Completed, Failed, or Cancelled status
- **THEN** a row SHALL be inserted into `subagent_transcripts` with id, session_id, parent_id, label, status, system_prompt, user_prompt, started_at, finished_at, total_tokens, error_message (if any), summary, and--on failure--`failure_diagnostics`, `root_cause`, and `retry_history`

#### Scenario: Events batch-written on subagent completion
- **WHEN** a subagent completes
- **THEN** all events (thought, action, tool_result, error) from the subagent's execution SHALL be inserted into `subagent_events` in a single transaction

#### Scenario: Idempotent migration adds diagnostics columns to existing databases
- **WHEN** the transcript store is opened on an existing database created before the diagnostics columns existed
- **THEN** the system SHALL add `failure_diagnostics`, `root_cause`, and `retry_history` columns via `ALTER TABLE ADD COLUMN` only if they are not already present (checked via `PRAGMA table_info`), without losing existing data

#### Scenario: Old rows degrade gracefully
- **WHEN** a transcript row predates the diagnostics columns (columns NULL)
- **THEN** reads SHALL surface `root_cause` as `Unknown` and empty `retry_history`/`failure_diagnostics` rather than erroring

## ADDED Requirements

### Requirement: Failure diagnostics persistence
On subagent failure, the system SHALL persist the structured failure diagnostics (root cause, failed tool-call sequence, failed-round context, retry history) into the `subagent_transcripts` diagnostics columns in the same transaction that writes the header row, so that CLI and SSE replay can retrieve them without re-running the subagent.

#### Scenario: Diagnostics written with header on failure
- **WHEN** a subagent fails and its transcript header is written
- **THEN** `failure_diagnostics`, `root_cause`, and `retry_history` SHALL be populated in the same transaction

#### Scenario: Diagnostics absent on success
- **WHEN** a subagent completes successfully
- **THEN** the diagnostics columns SHALL be NULL/empty
```

