## Why

`micro_compact_messages`（`src/agent/runtime/compaction.rs:165`）在 agent loop 每一轮**无条件**执行：最近 3 条工具结果之外全部替换为 `[Previous: used X]` 占位符（`loop_.rs:229`）。这带来两个真实代价：

1. **缓存系统性 miss**：占位翻转点随每轮交互前移（第 N 轮的倒数第 3 条结果在第 N+1 轮翻转为 marker），请求前缀从翻转点开始分歧。Anthropic 前缀缓存（system 断点 `conversions.rs:103` + 倒数第二消息断点 `conversions.rs:153`）每轮只能命中到翻转点——**最近 3-4 个交互（往往含最大的工具输出）永远无法命中**，每轮全价重算 + 1.25x cache write。OpenAI 隐式前缀缓存同理。
2. **错误信息丢失阻断自纠**：失败的 `apply_patch` 调用（错误码/上下文不匹配原因）一旦滑出保留窗口即被压成 `[Previous: used apply_patch]`，模型无法读取失败原因进行重试——已在真实会话中观测到（apply_patch 三连败后无法自纠，只能换工具绕行）。

## What Changes

- **Lazy 压实**：请求视图默认逐字（verbatim）保留历史；仅当校准后的 token 估算超过水位线（窗口 60% − max_tokens）时，一次性推进「压实前沿」（sticky frontier）——前沿之前的最旧工具结果压成 marker，之后逐字保留。前沿单调前进，两次触发之间视图 append-only，前缀缓存全命中。
- **错误豁免**：压实工具结果时嗅探错误载荷（executor 统一格式 `{"success":false,"error":{message,code}}`、字符串形态 `{"success":false,"error":"..."}`、hook 拦截纯文本 `Tool 'X' blocked by hook: ...`），失败结果保留截断错误摘要：`[Previous: used X; error: <msg>]`；成功结果保持裸 marker。
- 全量 LLM 摘要压缩（`needs_compaction`）改为在实际发送视图（前沿后）上评估——行为不变、触发更少更晚。
- `file_read`/`read_file` 永久豁免语义保持；413 应急路径 `fallback_micro_compact` 与摘要器输入 `prepare_compaction_transcript` 不动。
- 会话持久化不新增字段（frontier 运行时态，恢复后默认 0 = 逐字，安全）。

## Capabilities

### New Capabilities

- `context-compaction-policy`: 定义 agent 请求视图的工具结果压实策略——verbatim 默认、水位触发的前沿推进、错误摘要保留、file_read 豁免。

### Modified Capabilities

<!-- 无。agent-runtime-engine spec 无 compaction 相关需求，本 change 以新能力承载。 -->

## Impact

- **数据流**：`loop_.rs` 请求视图构建点（`:227-241`）改为 frontier 分段；`LoopTurnState` 增加 `micro_compact_frontier` 字段。
- **纯函数层**：`compaction.rs` 新增视图构建/前沿推进/错误嗅探辅助函数（可完整单测）；`micro_compact_messages` 本体语义不变（供应急路径与摘要器继续使用）。
- **回归风险**：低——视图只影响 API 请求内容，history 与会话持久化不变；marker 格式向后兼容（新增 `; error:` 后缀仅在失败结果出现）。
- **收益**：稳态下前缀缓存全命中（省最近几轮的重复输入费用）；失败工具调用可自纠，减少整轮重试与换工具绕行。
