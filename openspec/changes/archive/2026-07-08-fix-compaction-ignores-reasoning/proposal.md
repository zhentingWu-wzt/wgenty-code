## Why

长对话运行到一定长度后，Ark（`glm-latest` via `https://ark.cn-beijing.volces.com/api/coding/v3`，≈128K tokens 窗口）返回 `InvalidParameter: Invalid request body` 拒绝请求，agent 中断。根因是自动压缩（compaction）这道防线失效：`needs_compaction` 只统计 message `content`（仅占真实请求 ~32%），忽略 `reasoning_content`（44%，thinking 模型最大开销）和 `tool_calls.arguments`（18%），加之 `MAX_ESTIMATED_TOKENS = 800_000` 远超真实窗口，压缩永不触发。近期 `remove-token-budget-limits` 移除 token 预算硬截断后，压缩成为唯一防线，失效即溢出。

## What Changes

- `needs_compaction`（`src/tui/agent/compaction.rs`）的字符估算从只累加 `content` 扩展为 `content + reasoning_content + tool_calls.arguments`，反映真实请求大小（均为 `ChatMessage` 已有字段，无需改函数签名或传 tools）。
- `MAX_ESTIMATED_TOKENS`（`src/tui/agent/mod.rs`）从 `800_000` 降到 `80_000`，使压缩在真实窗口附近（~320K 全量字符 ≈95K 真实 tokens）触发，128K 窗口下留 ~30K 余量。

## Capabilities

### New Capabilities

（无）

### Modified Capabilities

（无 — 压缩触发阈值与字符估算是内部实现细节，不改变任何已有 spec 的验收场景）

## Impact

- 代码：`src/tui/agent/compaction.rs`（`needs_compaction`）、`src/tui/agent/mod.rs`（`MAX_ESTIMATED_TOKENS` 常量）。2 文件，同模块，单关注点。
- 行为：长对话会更早、更准确地触发压缩，避免上下文溢出导致 Ark 拒绝请求；短对话无影响（远低于阈值）。
- 风险：阈值 80K 偏保守，长会话压缩频率上升，但优于溢出崩溃。估算仍基于字符（`/4`），未用真实 `prompt_tokens`，已通过 ~30K 余量预留吸收误差。
