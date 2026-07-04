---
comet_change: split-api-and-subagent-result-relief
role: technical-design
canonical_spec: openspec
---

# API Module Split & Subagent Result Delivery Relief — Design Spec

**Date**: 2026-07-04
**Status**: Design approved (brainstorm confirmed — B3 hybrid)
**Scope**: `src/api/` module split (refactor) + `src/teams/subagent_mailbox.rs` large-result delivery redesign (behavior change)
**Canonical spec**: `openspec/changes/split-api-and-subagent-result-relief/specs/subagent-result-delivery/spec.md`

## Problem

两个独立问题需要处理:

1. **`src/api/mod.rs` 臃肿**:原约 670 行,error 处理(`wrap_network_error`/`format_api_error`)、wire-format 类型(`ChatMessage`/`ToolDefinition`/`ChatResponse`/`StreamChunk` 等)与 `ApiClient` 实现混在单文件,职责混杂、难维护。

2. **subagent 大结果交付两难**:旧设计把大结果(>4000 字符)替换为 200 字符摘要返回父 agent —— 丢失关键细节(代码探索、架构分析、多步研究结论)。工作区曾改为"全量返回",但又会把大结果全量塞进父 agent 上下文,在 subagent 频繁产大结果的场景下显著膨胀 token。需要在"不丢细节"与"控制 token"之间建立平衡机制。

## Design Goals / Non-Goals

**Goals**
- (A) 拆分 `src/api/mod.rs` 为 `error.rs` + `types.rs`,降低单文件体积,外部接口零变化。
- (B) subagent 大结果交付:"不丢细节" + "控制父 agent 上下文 token 成本",通过分档机制平衡两者。

**Non-Goals**
- 不改 api 公共接口(re-export 保持 `crate::api::*` 路径)。
- 不改 mailbox JSONL 文件格式(`StoredResult` 结构不变)。
- 不改 subagent 调度逻辑。
- 不改 `do_auto_compact` / `micro_compact`(B3 不依赖 compaction 改造)。
- 不拆 `anthropic`/`provider`/`token_counter` 子模块。

## 现状分析(代码调查事实)

brainstorming 阶段已确认以下事实:

1. **`do_auto_compact`**(`src/tui/agent/compaction.rs:107`)是全局 LLM summary,对整段历史做摘要,不识别特定 subagent 结果,不引用 mailbox 磁盘副本。→ B2(compaction 降级)方案落地成本高、风险大,排除。
2. **`to_compact()`** 是 dead code —— grep 全仓无外部调用方。它本应是 compaction 降级入口但从未接入。→ 删除,无功能影响。
3. **`micro_compact`**(`compaction.rs:42`)是局部替换旧 tool results 的机制(保留最近 3 条 + 所有 file_read,其余替换为 `[Previous: used <tool>]`)。B3 不依赖它。
4. **调用方**:`src/tools/meta/task.rs:558`(后台 subagent)、`:704`(同步 subagent)调 `offload_if_large(...).to_content()`。`to_content()` 是实际进父 agent 上下文的唯一接口。

**当前工作区状态(`subagent_mailbox.rs`)**:已被改成"全量返回"版本 —— `SubagentResponse::{Inline, Offloaded}` 两档,`Offloaded` 全量 `content` + 磁盘路径,`to_compact()` 委派 `to_content()`(不截断)。此版本解决了"丢失细节"但未解决"token 膨胀",B3 在此基础上新增第三档。

## Change A: `src/api` 模块拆分(纯重构)

### 策略
- `mod.rs` → `error.rs`(error 处理 `wrap_network_error`/`format_api_error`)+ `types.rs`(wire-format 类型 `ToolDefinition`/`ChatMessage`/`ChatRequest`/`ChatResponse`/`StreamChunk` 等)。
- `mod.rs` 声明 `pub mod error; pub mod types;` 并通过 glob re-export 保持旧路径:
  ```rust
  pub use types::*;
  pub(crate) use error::*;
  ```
- 外部接口零变化:`crate::api::ChatMessage`、`crate::api::format_api_error` 等旧路径继续可用。

### 为何 glob 而非显式 `pub use`
- error helpers 是 `pub(crate)`,无法用 `pub use` 再导出(会警告且无法暴露给 `crate::api::format_api_error` 调用方,如 `daemon/handlers.rs`)。
- 显式 `use error::{...}` 会以私有导入 shadow 掉 glob re-export,破坏 `crate::api::format_api_error` 调用点。
- `pub(crate) use error::*;` 一行同时完成"本模块内导入"与"crate 级路径再导出"。
- `mod.rs:11-17` 注释已记录该理由。

### 落地状态(工作区已完成)
- `src/api/error.rs`(5166 bytes)、`src/api/types.rs`(8471 bytes)已创建,含迁移的单元测试。
- `src/api/mod.rs` 已从 ~670 行降至 381 行,re-export 已就位(`mod.rs:6-17`)。
- build 阶段 task 1.4 验证 `cargo build && cargo test --lib && cargo clippy --lib` 零行为变化、旧 `crate::api::*` 引用仍编译。

## Change B: subagent 大结果交付 — B3 混合方案

### 双阈值与常量
| 常量 | 值 | 语义 |
|------|----|------|
| `MAX_INLINE_RESULT_LEN` | 4000 | 磁盘持久化阈值(超此写磁盘副本)—— 已存在,语义不变 |
| `MAX_FULL_INLINE_LEN` | 8000 | 全量进上下文上限(新常量)—— 超此走摘要 |
| `SUMMARY_HEAD_LEN` | 1500 | 摘要头部长度(新常量)—— `content.chars().take(1500)` |

### 三档 `SubagentResponse`
```rust
pub enum SubagentResponse {
    /// ≤4000: 全量进上下文,不写磁盘。
    Inline { content: String },
    /// 4000–8000: 全量进上下文 + 磁盘副本。
    Offloaded { content: String, mailbox_path: PathBuf, content_len: usize },
    /// >8000: 头 1500 字摘要 + 磁盘路径,不进全文。
    Summarized { summary: String, mailbox_path: PathBuf, content_len: usize },
}
```

### `offload_if_large()` 分档逻辑
```rust
pub fn offload_if_large(&self, subagent_type, description, session_id, content) -> SubagentResponse {
    let len = content.len();
    if len <= MAX_INLINE_RESULT_LEN {
        return SubagentResponse::Inline { content: content.to_string() };
    }
    // >4000: 先尝试磁盘持久化
    match self.store(subagent_type, description, session_id, content) {
        Ok(path) => {
            if len <= MAX_FULL_INLINE_LEN {
                // 4000–8000: 全量 + 磁盘
                SubagentResponse::Offloaded {
                    content: content.to_string(), mailbox_path: path, content_len: len,
                }
            } else {
                // >8000: 头摘要 + 磁盘
                let summary: String = content.chars().take(SUMMARY_HEAD_LEN).collect();
                SubagentResponse::Summarized { summary, mailbox_path: path, content_len: len }
            }
        }
        Err(e) => {
            // 磁盘失败:降级 Inline(全量,无副本,logged)—— 完整性优先于 token 控制
            tracing::warn!(
                error = %e,
                "Failed to persist subagent result; returning full inline (no recovery copy)"
            );
            SubagentResponse::Inline { content: content.to_string() }
        }
    }
}
```

### `to_content()` 三档输出
- **Inline** → `content`
- **Offloaded** → `content` + footer:
  ```
  <content>

  ---
  [Full result ({content_len} chars) persisted at: `{path}` for recovery]
  ```
- **Summarized** → `summary` + footer:
  ```
  <summary>

  ---
  [Summary only. Full result ({content_len} chars) at `{path}` — file_read for details]
  ```

### 磁盘持久化失败降级
- 任何 `>4000` 的结果,若 `store()` 失败,降级为 `Inline { content }`(全量返回,无磁盘副本,`tracing::warn!` 记录)。
- **即使 `>8000` 也全量返回**:磁盘失败时内容完整性优先于 token 控制 —— 丢内容不可逆,token 膨胀可由后续 compaction 缓解。这是 spec requirement 4(Persistence failure does not lose content)的直接实现。

### `to_compact()` 删除
- 已确认 dead code(grep 无外部调用方)。
- 删除该方法 + 其两个测试(`test_to_compact_returns_full_content`、`test_offloaded_to_compact_returns_full_content`)。
- compaction 走 `do_auto_compact`(全局 LLM summary),不调 `to_compact`。

### 调用方不变
- `src/tools/meta/task.rs:558`(后台)、`:704`(同步)仍 `offload_if_large(...).to_content()` —— `to_content()` 签名不变,调用方零改动。

### 不涉及 compaction 改造
- B3 不改 `do_auto_compact` / `micro_compact`。token 缓解完全由 `MAX_FULL_INLINE_LEN=8000` 上限 + `Summarized` 档实现,与 compaction 解耦。

## 与 spec requirements 的映射

| Spec Requirement | B3 实现 |
|---|---|
| **R1: Large results accessible without loss** | `Offloaded`(4000–8000)全量 inline;`Summarized`(>8000)磁盘存全文,父 agent 可 `file_read` 恢复;磁盘失败降级 `Inline` 全量。无 200 字短摘要作为唯一表示。 |
| **R2: Delivery controls token cost** | `MAX_FULL_INLINE_LEN=8000` 上限:`>8000` 不全量 inline,走 `Summarized` 档(头 1500 字)。token 有界。 |
| **R3: Disk persistence for recovery** | `>4000` 即 `store()` 写 JSONL mailbox,`Offloaded`/`Summarized` 均携带 `mailbox_path` 并在 footer 通信给父 agent。 |
| **R4: Persistence failure does not lose content** | `store()` 失败降级 `Inline { content }`(全量,无截断),`tracing::warn!` 记录。 |

## Open Questions 的解决(brainstorm 结论)

| Open Question | Resolution |
|---|---|
| `to_compact()` 是否 dead code?是否应接入 compaction? | 是 dead code,删除。compaction 走 `do_auto_compact`,不需要 `to_compact` 入口。 |
| compaction 能否识别并替换特定 subagent 结果? | 不需要 —— B3 不依赖 compaction,token 缓解由 `MAX_FULL_INLINE_LEN` 上限实现。 |
| B1 摘要生成方式:头 N 字 vs 结构化提取? | B3 的 `Summarized` 档用头 1500 字前缀(`content.chars().take(1500)`)。结构化提取需 LLM 调用,成本高且引入非确定性,本期不做。 |

## Risks / Trade-offs

- **4000–8000 区间全量进上下文**:信息完整但近期 token 压力。可接受 —— 8000 上限封顶,且此区间有磁盘副本供 compaction 后恢复。
- **>8000 头 1500 字摘要**:仍可能漏后文细节。靠 footer 的 `file_read for details` 提示缓解 —— 父 agent 不读则漏(同旧设计风险,但摘要更长 1500 vs 200 + 明确恢复提示)。
- **磁盘失败时 >8000 全量返回**:违反"token 可控"但满足"不丢内容"(spec R4 优先)。磁盘失败罕见,可接受。
- **删除 `to_compact`**:清理 dead code,无功能影响;若未来计划接入 compaction 需重新引入。本期 compaction 不需要它。
- **glob re-export 可能掩盖新类型导出意图**(Change A):可接受,`mod.rs:11-17` 注释已说明 glob 选择理由。

## Test Strategy

### Change A(api 拆分)
- 迁移的单元测试随 `error.rs`/`types.rs` 移动,`cargo test --lib` 全绿。
- `cargo build` 验证旧 `crate::api::*` 路径仍编译(`daemon/handlers.rs` 的 `crate::api::format_api_error` 等)。
- `cargo clippy --lib` 无新警告。

### Change B(subagent_mailbox B3)
**保留/调整**:
- `test_small_result_stays_inline`(≤4000 → `Inline`)。
- `test_large_result_offloaded_with_full_content`(5000,4000–8000 → `Offloaded`,全量 + 磁盘)。
- `test_offloaded_to_content_returns_full_result`(5000,`to_content` 含全文 + footer)。
- `test_inline_to_content_unchanged`。

**删除**:
- `test_to_compact_returns_full_content`、`test_offloaded_to_compact_returns_full_content`(`to_compact` 已删)。

**新增**:
- `test_very_large_result_summarized`(`"A".repeat(9000)` → `Summarized`,`summary` 为头 1500 字,`mailbox_path` 存在,`content_len == 9000`)。
- `test_summarized_to_content_has_summary_and_path`(`to_content` 含头 1500 字 + `[Summary only. ... file_read for details]` footer + 磁盘路径)。
- `test_summarized_summary_is_head_prefix`(确认 `summary == content.chars().take(1500).collect::<String>()`)。
- `test_disk_persistence_failure_degrades_to_inline`(`>8000` + `store` 失败 → `Inline` 全量,无副本;用不可写目录模拟失败)。
- `test_boundary_4000_inline_vs_offloaded`(`len == 4000` → `Inline`;`len == 4001` → `Offloaded`)。
- `test_boundary_8000_offloaded_vs_summarized`(`len == 8000` → `Offloaded`;`len == 8001` → `Summarized`)。

### Spec scenario 覆盖
- **R1**: `test_large_result_offloaded_with_full_content` + `test_very_large_result_summarized`(磁盘可恢复)。
- **R2**: `test_very_large_result_summarized` + `test_boundary_8000_offloaded_vs_summarized`(不全量 inline)。
- **R3**: `test_offloaded_to_content_returns_full_result` + `test_summarized_to_content_has_summary_and_path`(footer 含路径)。
- **R4**: `test_disk_persistence_failure_degrades_to_inline`(全量 + logged)。

## Implementation Surface

| 文件 | 改动 | Change |
|------|------|--------|
| `src/api/mod.rs` | 拆分后剩 `ApiClient` + re-export(已在工作区) | A |
| `src/api/error.rs` | 新建,迁入 error 处理(已在工作区) | A |
| `src/api/types.rs` | 新建,迁入 wire-format 类型(已在工作区) | A |
| `src/teams/subagent_mailbox.rs` | 新增 `MAX_FULL_INLINE_LEN`/`SUMMARY_HEAD_LEN` 常量;`SubagentResponse::Summarized` 变体;`offload_if_large` 三档分档;`to_content` 三档输出;删除 `to_compact`;更新测试 | B |
| `src/tools/meta/task.rs` | 无改动(`to_content` 签名不变) | B |
| `src/tui/agent/compaction.rs` | 无改动(B3 不依赖 compaction) | B |

## Build Order

1. **Change A 验证**(task 1.x):工作区代码已就位,跑 `cargo build && cargo test --lib && cargo clippy --lib` 确认零行为变化。
2. **Change B 实现**(task 2.x):
   - 2.1 已完成(brainstorm 选定 B3)。
   - 2.2 已完成(`to_compact` 确认 dead code)。
   - 2.3 调整 `subagent_mailbox.rs`(三档 + 常量 + `to_content` + 删 `to_compact`)。
   - 2.4 调用方确认无改动。
   - 2.5 跳过(B3 非 B2,不改 compaction)。
   - 2.6 新增/调整测试。
   - 2.7 `cargo test` 验证 spec scenarios。
