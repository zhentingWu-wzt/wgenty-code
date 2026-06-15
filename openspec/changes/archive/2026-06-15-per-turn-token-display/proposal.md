## Why

当前状态栏的 token 展示统计的是 API 返回的 `total_tokens` 全程累积值，混入了系统提示词、工具定义、工具结果、对话历史等并非用户直接产生的 token 消耗。用户无法直观感知自己每次输入和模型每次输出的实际 token 规模。改为按 turn 分离展示用户输入 token 和模型输出 token，剔除系统/工具噪音，让用户看到真正属于自己的消耗。

## What Changes

- **TokenCounter 扩展**：新增 `turn_input` 和 `turn_output` 两个原子计数器，分别追踪当前 turn 的用户输入 token 估算值和模型输出 token（`completion_tokens`），保留原有 `used` 计数器用于预算控制不变
- **用户输入拦截**：在 `AgentLoop::process_input_inner` 中，用户消息推入对话历史前，用 `chars/4` 估算 token 数并累加到 `turn_input`
- **输出 token 累加**：在 `AgentLoop::run_agent_loop` 中，每轮 LLM 调用后将 `usage.completion_tokens` 累加到 `turn_output`（替代当前的 `usage.total_tokens`）
- **Turn 重置**：每个新 turn 开始时（`process_input` 入口），将 `turn_input` 和 `turn_output` 归零
- **状态栏显示变更**：从 `↓ Nk tokens`（单值）改为 `↑ N · ↓ Mk`（分别展示输入/输出），闲置时保留上 turn 值；token=0 时不展示对应部分
- **TypeScript Ink 前端**：暂不修改

## Capabilities

### New Capabilities

- `per-turn-token-display`: 按 turn 分离展示用户输入 token 和模型输出 token，剔除系统提示词、工具定义、工具结果、对话历史等非用户直接产生的 token

### Modified Capabilities

<!-- 无现有 spec 受影响 -->

## Impact

| 文件 | 变更 |
|------|------|
| `src/api/token_counter.rs` | 新增 `turn_input`/`turn_output` 字段及 `add_input`/`add_output`/`reset_turn` 方法 |
| `src/tui/agent/mod.rs` | `process_input` 入口重置 turn 计数器；`process_input_inner` 中估算用户输入 token |
| `src/tui/agent/core.rs` | `run_agent_loop` 中将 `usage.completion_tokens` 替换 `usage.total_tokens` 用于显示 |
| `src/tui/components/status.rs` | `render` 签名扩展为接收 `(input_tokens, output_tokens)`，`format_tokens` 改为 `↑ N · ↓ Mk` |
| `src/tui/app/render.rs` | 读取 `turn_input`/`turn_output` 传入 status render |
