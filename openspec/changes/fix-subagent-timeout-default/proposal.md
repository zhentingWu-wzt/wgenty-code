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
