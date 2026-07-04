# Brainstorm Summary

- Change: split-api-and-subagent-result-relief
- Date: 2026-07-04

## 确认的技术方案

### Change A(src/api 模块拆分)
- 已定案:`mod.rs` → `error.rs` + `types.rs`,glob re-export(`pub use types::*; pub(crate) use error::*;`)保持 `crate::api::*` 接口。纯重构,代码已在工作区。

### Change B(subagent 大结果交付)— B3 混合,已确认
- **双阈值**:
  - `MAX_INLINE_RESULT_LEN = 4000`(磁盘持久化阈值;语义:超此写磁盘副本)
  - `MAX_FULL_INLINE_LEN = 8000`(全量进上下文上限;新常量)
- **三档行为**:
  - ≤4000:`Inline { content }` —— 全量进上下文,不写磁盘
  - 4000–8000:`Offloaded { content, mailbox_path, content_len }` —— 全量进上下文 + 磁盘副本
  - >8000:`Summarized { summary, mailbox_path, content_len }` —— 头 1500 字摘要 + 磁盘路径,不进全文
- **`to_content()`**:
  - Inline → content
  - Offloaded → content + footer `[Full result ({len} chars) persisted at: \`{path}\` for recovery]`
  - Summarized → summary + footer `[Summary only. Full result ({len} chars) at \`{path}\` — file_read for details]`
- **`offload_if_large()`**:按大小分档;磁盘持久化失败降级 `Inline { content }`(不丢内容,无副本,logged)
- **`to_compact()`**:dead code,删除(compaction 走 `do_auto_compact`,不调它)
- **调用方**:`task.rs:558,704` 不变(仍 `offload_if_large(...).to_content()`)
- **不涉及** `micro_compact` / `do_auto_compact` 改造

## 代码调查发现(已确认事实)
1. `do_auto_compact`(`compaction.rs:107`)是全局 LLM summary,不识别特定 subagent 结果,不引用 mailbox 磁盘副本。
2. `to_compact()` 是 dead code(grep 无外部调用方)。
3. `micro_compact`(`compaction.rs:42`)是局部替换旧 tool results 机制。B3 不用它。
4. 调用方:`task.rs:558`(后台)、`704`(同步)调 `offload_if_large(...).to_content()`。

## 关键取舍与风险
- 4000–8000 区间全量进上下文:信息完整但近期 token 压力(可接受,因 8000 上限)。
- >8000 头 1500 字摘要:仍可能漏后文细节,靠"file_read 提示"缓解。父 agent 不读则漏(同旧设计风险,但摘要更长 1500 vs 200 + 明确提示)。
- 删除 `to_compact`:清理 dead code,无功能影响。

## 测试策略
- ≤4000 / 4000–8000 / >8000 三档 `to_content` 输出正确
- 磁盘持久化失败降级 `Inline`(全量,无副本,logged)
- `Summarized` 的 `to_content` 含摘要 + 路径 + file_read 提示
- 现有测试更新(旧 `summarize`/`SUMMARY_PREFIX_LEN` 已删,新增头摘要测试)

## Spec Patch
- 无。现有 4 requirements(不丢细节 / token 可控 / 磁盘恢复 / 持久化失败降级)已抽象覆盖 B3。
