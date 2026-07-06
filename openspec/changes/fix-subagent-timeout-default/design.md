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
