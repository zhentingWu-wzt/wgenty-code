# Brainstorm Summary

- Change: per-turn-token-display
- Date: 2026-06-15

## 确认的技术方案

方案 A（TokenCounter 内聚扩展）：在现有 `TokenCounter` 上新增 `turn_input`/`turn_output` 两个 `Arc<AtomicUsize>` 字段，通过 `add_input`/`add_output`/`reset_turn` 方法操作。AgentLoop 在 `process_input` 入口重置 turn，在 `process_input_inner` 中估算用户输入（chars/4），在 `run_agent_loop` 每轮 LLM 调用后累加 `completion_tokens`。状态栏渲染签名从 `tokens_used: usize` 改为 `input_tokens: usize, output_tokens: usize`，显示格式 `↑ N · ↓ Mk`。

## 关键取舍与风险

- chars/4 估算中文场景偏高 ~2x，用户已知悉可接受
- TokenCounter 变成"预算+展示"混合体，但结构紧凑改动最小
- 子代理 token 天然隔离（不操作主 TokenCounter）
- 预算控制保留原有 `used` 字段不变

## 测试策略

- TokenCounter 单元测试：原子操作正确性
- 手动集成验证：状态栏格式、多轮重置

## Spec Patch

无
