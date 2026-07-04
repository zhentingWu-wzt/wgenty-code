## Context

- **`src/api/mod.rs` 现状**:约 670 行,error 处理(`wrap_network_error`/`format_api_error`)、wire-format 类型(`ChatMessage`/`ToolDefinition`/`ChatResponse`/`StreamChunk` 等)、`ApiClient` 实现混杂在同一文件,职责不清、难维护。
- **`src/teams/subagent_mailbox.rs` 现状**:大结果(>`MAX_INLINE_RESULT_LEN=4000`)offload 到 JSONL mailbox 文件,父 agent 收到 200 字符摘要 + 磁盘路径引用(`SubagentResponse::Offloaded { summary, mailbox_path, content_len }`)。`to_content()` 返回摘要式,`to_compact()` 截断大 inline 结果。
- **工作区已有改动**:(A) api 拆分已完成(纯重构);(B) mailbox 已被改成"全量返回 + `to_compact` 不截断 + 删除 `summarize()`",但用户要求"需要缓解机制",全量返回不是最终方案,需重新设计。
- **compaction 机制存在**:`src/tui/agent/compaction.rs:107` `do_auto_compact`(agent loop 自动触发)+ `src/tools/meta/compact.rs` Compact 工具,会 archive transcript 并以 summary 替换。这是"compaction 时降级"方案的落地基础。
- **接口现状**:`to_content()` 是实际进父 agent 上下文的接口(`src/tools/meta/task.rs:558,749` 调 `offload_if_large(...).to_content()`);`to_compact()` 无外部调用方(grep 无结果),疑似 dead code 或本该接入 compaction 但未接上 —— design 阶段需查清。

## Goals / Non-Goals

**Goals:**
- (A) 拆分 `src/api/mod.rs` 为 `error.rs` + `types.rs`,降低单文件体积,外部接口零变化。
- (B) subagent 大结果交付:"不丢细节" + "控制父 agent 上下文 token 成本",通过某种缓解机制平衡两者。

**Non-Goals:**
- 不改 api 公共接口(re-export 保持 `crate::api::*` 路径)。
- 不改 mailbox JSONL 文件格式。
- 不改 subagent 调度逻辑。
- 不拆 `anthropic`/`provider`/`token_counter` 子模块。

## Decisions

### Decision A: api 模块拆分策略(已定)
- `mod.rs` → `error.rs`(error 处理)+ `types.rs`(wire-format 类型)。
- 通过 `pub use types::*; pub(crate) use error::*;` 保持 `crate::api::ChatMessage`、`crate::api::format_api_error` 等旧路径可用。
- **理由**:纯代码移动,glob re-export 保证零行为变化。
- **备选**:逐个 `pub use` 显式列出 → 更精确但维护成本高,且 `pub(crate)` 项无法用 `pub use` 再导出(会警告且无法暴露给 `crate::api::format_api_error` 调用方)。选 glob。

### Decision B: subagent 大结果交付机制(候选,待 design 阶段 brainstorming 定案)

三个候选方向:

- **B1 按需加载**:`to_content()` 返回结构化摘要(关键结论/数据/建议,~500–1000 字)+ 磁盘路径,父 agent 需要细节时 `file_read` 完整内容。
  - 优点:省 token(只摘要进上下文);磁盘副本本就存在。
  - 缺点:依赖父 agent 主动判断"需要读全文",不读则漏细节(与旧设计同风险,但摘要可更结构化)。
- **B2 compaction 时降级**:`to_content()` 全量返回(短期信息完整),`do_auto_compact` 触发时识别上下文中的大 subagent 结果,替换为磁盘引用 + 摘要。
  - 优点:短期全量可用(信息完整),长期压缩后省 token。
  - 缺点:需改 compaction 逻辑识别结果标记,compaction 是核心机制,改动风险高。
- **B3 混合**:全量进上下文但有上限,超上限走 B1 按需。
  - 优点:多数情况全量,极大结果按需。
  - 缺点:双阈值,复杂度中等。

**待 comet-design 阶段 brainstorming 选定**,需先查清 `to_compact()` 是否 dead code、compaction 是否可 hook 结果替换。

## Risks / Trade-offs

- [A: glob re-export 可能掩盖新类型导出意图] → 可接受,`mod.rs` 注释已说明 glob 选择理由。
- [B1: 父 agent 不读全文则漏细节] → 摘要需足够 informative(结构化结论而非头 N 字)。
- [B2: 改 compaction 风险高] → compaction 是核心机制,需充分测试 + 回归。
- [B: 已写的全量返回代码需调整] → design 定案后改,已有磁盘副本逻辑可复用。
- [B: 任意方案都改变父 agent 看到的内容] → 需同步更新 `src/tools/meta/task.rs` 调用方语义。

## Open Questions

- `to_compact()` 是否 dead code?是否本该接入 compaction 作为降级点(B2 的天然入口)?
- compaction 机制能否识别并替换上下文中的特定 subagent 结果(需结果埋标记,如 footer 的 path)?
- B1 摘要的生成方式:头 N 字 vs 结构化提取(结论/数据/建议)?
