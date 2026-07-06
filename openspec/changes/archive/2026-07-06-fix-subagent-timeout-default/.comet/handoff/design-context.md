# Comet Design Handoff

- Change: fix-subagent-timeout-default
- Phase: design
- Mode: compact
- Context hash: c4e6252b0f66e1f0e1be32d8a8f44803a385d2824b92e1a44aab2ebe99c988f7

Generated-by: comet-handoff.sh

OpenSpec remains the canonical capability spec. This handoff is a deterministic, source-traceable context pack, not an agent-authored summary.

## openspec/changes/fix-subagent-timeout-default/proposal.md

- Source: openspec/changes/fix-subagent-timeout-default/proposal.md
- Lines: 1-40
- SHA256: 7764790b13a61df04893cc45871d33f5232b675755c03421028971a97d561781

```md
## Why

后台 subagent 执行约 4 分钟（240s）的任务时，在 tick 182 处被强制终止，主会话收到 `Subagent timed out after 240 seconds` 与部分结果。用户预期 subagent 能跑完整个任务并返回最终摘要，实际却"跑着跑着没结果"。

此外，主 Agent 循环存在一个硬编码的 60 分钟 per-turn 超时（`src/tui/agent/mod.rs` 的 `AGENT_LOOP_TIMEOUT = 3600s`），作为兜底机制它过于粗暴——当任务确实需要长时间运行（例如长 subagent 等待 + 多轮工具调用）时会误杀正常会话。用户明确要求移除此硬上限。

## 根因分析

### subagent 超时过紧

subagent 的硬超时 `agent.subagent.timeout_secs` 默认 240s（`src/config/agent.rs:76`），由 `tokio::time::timeout(timeout_duration, loop_future)` 对**整个循环**计时（`src/teams/subagent_loop.rs:771`）。该预算覆盖 LLM 推理 + 工具调用 + 实际任务全部耗时。

当任务本身需要 240s 时，初始 LLM 推理与多轮 `write_stdin` 输出收集的开销（约 50–60s）将总耗时推过 240s 上限，subagent 在 loop 跑到 ~182s 时被掐断，无法产出最终摘要。

### 主循环硬上限不合理

`AGENT_LOOP_TIMEOUT`（`src/tui/agent/mod.rs:99`）以 `tokio::time::timeout(3600s, process_input_inner)` 包裹整个 per-turn 循环。该值硬编码、不可配置，且覆盖 LLM 推理 + 全部工具调用 + subagent 等待。当 subagent 本身需要 30 分钟时，主循环 60 分钟上限形同虚设，且会在不恰当的时机切断会话。

## What Changes

- 将 `SubagentLimits::default().timeout_secs` 从 `240` 上调到 `1800`（30 分钟，`src/config/agent.rs:76`），给 LLM 推理与工具开销留出充足余量。
- **移除**主 Agent 循环的 `AGENT_LOOP_TIMEOUT` 硬上限（`src/tui/agent/mod.rs`）：`process_input` 直接调用 `process_input_inner`，不再套 `tokio::time::timeout`。主循环安全性由 `max_rounds`（默认 100，可配置）和 API 请求超时（`models.transport.timeout`，默认 120s）兜底。
- 同步 `WGENTY.md` 配置表中 `agent.subagent.timeout_secs` 的默认值 240 → 1800。

## 修复目标

- subagent 执行数分钟级任务时不再因默认超时被掐断，能跑完并返回最终摘要。
- 主 Agent 循环不再有硬编码的 per-turn 时间上限，长时间任务可正常完成。
- 文档默认值与代码保持一致。

## Impact

- **Code**:
  - `src/config/agent.rs`（`SubagentLimits::default` 一行：240 → 1800）
  - `src/tui/agent/mod.rs`（移除 `AGENT_LOOP_TIMEOUT` 常量 + `tokio::time::timeout` wrapper + `use std::time::Duration` 未用 import）
- **Docs**: `WGENTY.md` 配置表默认值同步。
- **User-visible behavior**:
  - 默认 subagent 超时从 4 分钟放宽到 30 分钟；用户仍可通过 `settings.json` 覆盖。
  - 主 Agent 循环不再有 per-turn 60 分钟硬上限。
- **Non-goals**: 嵌套 subagent 的 per-tool backstop（`src/teams/subagent_loop.rs:651-656`，300s）及其 "240s" 注释、`subagent_health.rs:426` 建议文案、`subagent-status-display` spec 示例中的 "240 seconds" 均不在本次范围，列为后续 follow-up。token 预算不动（默认 0=无限，已是最宽松）。
```

## openspec/changes/fix-subagent-timeout-default/design.md

- Source: openspec/changes/fix-subagent-timeout-default/design.md
- Lines: 1-23
- SHA256: 186d300a974d6c97a0ec39ce20d063c96405066c00601bae8c6fde49b386ae9f

```md
## 修复方案

### 方案 A：上调默认 `timeout_secs` 240 → 1800

将 `SubagentLimits::default().timeout_secs` 改为 1800（30 分钟），使默认预算覆盖数分钟级任务并保留充足的 LLM/工具开销余量。

### 方案 B：移除主循环 `AGENT_LOOP_TIMEOUT` 硬上限

移除 `src/tui/agent/mod.rs` 中 `process_input` 的 `tokio::time::timeout(3600s, ...)` wrapper，改为直接 `await`。同时移除因此变为未使用的 `use std::time::Duration` import。

### 取舍

- **为何 1800s（30 min）**：用户原始任务 240s + ~60s 开销余量 + 未来数分钟级任务缓冲。30 分钟在可接受的 subagent 等待区间内，且与用户明确要求的"30 分钟"一致。
- **为何移除主循环硬上限而非改为可配置**：用户明确表示"不应该有这些限制"。主循环已有 `max_rounds`（默认 100，可配置）防止无限循环，API 请求级有 `models.transport.timeout`（120s）防止单次请求挂死，足够兜底。保留一个不可配置的 per-turn 时间墙反而会在长任务场景误杀正常会话。
- **为何不调 per-tool backstop（300s）**：该 backstop（`src/teams/subagent_loop.rs:651-656`）仅作用于嵌套 subagent（subagent 调 task/delegate），用户原始问题为顶层 subagent，不受其管辖。修改它会将改动扩到 3+ 文件，超出 hotfix 边界，故列为 follow-up。
- **为何不动 token 预算**：默认 `subagent_default_k=0` 即无限，本就不是本次超时的约束。

### 验证

- `cargo build` / `cargo clippy --lib -- -D warnings` 通过。
- 确认无测试断言 `timeout_secs == 240` 或 `timeout_secs == 600`（已 grep 确认）。
- 确认 `src/tui/agent/mod.rs` 不再存在 `AGENT_LOOP_TIMEOUT` 或 `Duration` import。
- 根因消除：`src/config/agent.rs` 中 `timeout_secs: 240` 不再存在。
```

## openspec/changes/fix-subagent-timeout-default/tasks.md

- Source: openspec/changes/fix-subagent-timeout-default/tasks.md
- Lines: 1-21
- SHA256: d83ebc7295667b553b0ac345a09e4743a1fe1e7b9b7213dde538d3cbcda5962e

```md
## 1. 上调 subagent 默认超时

- [x] 1.1 在 `src/config/agent.rs` 将 `SubagentLimits::default()` 的 `timeout_secs` 改为 `1800`（原 240）。
- [x] 1.2 `cargo clippy --lib -- -D warnings` 零 warning。

## 2. 移除主循环硬上限

- [x] 2.1 在 `src/tui/agent/mod.rs` 移除 `AGENT_LOOP_TIMEOUT` 常量及 `tokio::time::timeout` wrapper，`process_input` 直接调用 `process_input_inner`。
- [x] 2.2 移除因此变为未使用的 `use std::time::Duration` import。
- [x] 2.3 更新 `process_input` doc comment（移除 "timed out" 描述）。

## 3. 同步文档

- [x] 3.1 在 `WGENTY.md` 配置表中将 `agent.subagent.timeout_secs` 默认值改为 `1800`。

## 4. 构建与验证

- [x] 4.1 `cargo build` 通过。
- [x] 4.2 `cargo clippy --lib -- -D warnings` 零 warning。
- [x] 4.3 `cargo test` 相关测试通过（509 lib passed, 35 subagent_evaluation passed, 0 failed）。
- [x] 4.4 根因消除检查：`src/config/agent.rs` 不再存在 `timeout_secs: 240`；`src/tui/agent/mod.rs` 不再存在 `AGENT_LOOP_TIMEOUT`。
```

## openspec/changes/fix-subagent-timeout-default/specs/subagent-result-delivery/spec.md

- Source: openspec/changes/fix-subagent-timeout-default/specs/subagent-result-delivery/spec.md
- Lines: 1-20
- SHA256: 24d3435ea6f9876ee4408ca64afdcebc2a487cdff9cdb9e65d897b7141d1a56c

```md
## ADDED Requirements

### Requirement: Failed subagent delivers structured error code and partial results
When a subagent fails (timeout, budget exhaustion, stuck detection, parse error, or max-rounds exceeded), the system SHALL return a structured `SubagentError` to the parent agent carrying (1) a categorized error type that maps to a stable error-code string and (2) the subagent's last accumulated text snapshot as a partial result. The parent agent SHALL receive both the structured error code (via `ToolError::code`) and the partial work (via `ToolError::message`, which appends the partial result through `full_message()`), so it can salvage partial work and make informed retry/continue/abort decisions rather than receiving a bare error string with no recoverable output. The failed transcript SHALL record the same `full_message()` as its result snapshot.

#### Scenario: Subagent timeout delivers structured error code and partial results
- **WHEN** a subagent exceeds `agent.subagent.timeout_secs`
- **THEN** the parent agent SHALL receive a `ToolError` whose `code` is `subagent_timeout`
- **AND** the `ToolError::message` SHALL include any text the subagent accumulated before timing out, appended via `full_message()`'s "Partial results" section
- **AND** the failed transcript SHALL record the same `full_message()` as its result snapshot

#### Scenario: Budget exhaustion delivers budget_exceeded code and partial results
- **WHEN** a subagent exhausts its token budget
- **THEN** the parent agent SHALL receive a `ToolError` whose `code` is `budget_exceeded`
- **AND** the `ToolError::message` SHALL include the subagent's partial work accumulated before budget exhaustion

#### Scenario: Empty partial result does not append an empty segment
- **WHEN** a subagent fails and has no accumulated text snapshot, or the snapshot is empty/whitespace-only
- **THEN** `full_message()` SHALL return only the error message
- **AND** SHALL NOT append a "Partial results (before failure)" section
```

