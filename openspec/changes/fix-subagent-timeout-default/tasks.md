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
