## Why

两个独立问题需要处理:

1. `src/api/mod.rs` 把 error 处理(`wrap_network_error`/`format_api_error`)、wire-format 类型(`ChatMessage`/`ToolDefinition`/`ChatResponse`/`StreamChunk` 等)与 `ApiClient` 实现混在单文件(约 670 行),职责混杂、难维护。

2. subagent 大结果当前用 200 字符摘要替换完整内容返回父 agent —— 丢失关键细节(代码探索、架构分析、多步研究的结论)。但简单改为全量返回又会把大结果全量塞进父 agent 上下文,在 subagent 频繁产大结果的场景下显著膨胀 token。需要在"不丢细节"与"控制 token"之间建立平衡机制。

## What Changes

- **(A) src/api 模块拆分(纯重构)**:将 `mod.rs` 中的 error 处理移到 `src/api/error.rs`,wire-format 类型移到 `src/api/types.rs`。通过 `pub use types::*; pub(crate) use error::*;` 保持外部接口完全不变,`crate::api::ChatMessage`、`crate::api::format_api_error` 等旧路径继续可用。无行为变化。
- **(B) subagent 大结果交付重新设计(行为变更)**:重新定义大结果如何交付给父 agent —— 目标是"不丢细节 + token 可控"。已写的"全量返回 + `to_compact` 不截断"代码不是最终方案,将在 design 阶段确定缓解机制(候选:按需加载摘要+路径 / compaction 时降级引用磁盘 / 混合),并据此调整 `src/teams/subagent_mailbox.rs`。

## Capabilities

### New Capabilities
- `subagent-result-delivery`: 定义 subagent 结果如何交付给父 agent,涵盖大结果的持久化与 token 缓解机制(不丢细节 + 控制父 agent 上下文 token 成本)。

### Modified Capabilities
<!-- Change A 为纯重构,不改变 spec-level requirements;Change B 引入新 capability 而非修改现有。 -->
(无)

## Impact

- `src/api/{mod.rs, error.rs, types.rs}` —— 模块拆分(重构)
- `src/teams/subagent_mailbox.rs` —— 大结果交付行为变更
- `src/tools/meta/task.rs` —— mailbox 调用方(`offload_if_large(...).to_content()`),若 `to_content` 接口语义变化需同步
- `src/tui/agent/compaction.rs` —— design 阶段若选"compaction 降级"方案,需扩展 compaction 逻辑识别并替换上下文中的大 subagent 结果
- 无外部 API / 依赖变化(Change A 接口不变;Change B 是内部结果交付机制)
