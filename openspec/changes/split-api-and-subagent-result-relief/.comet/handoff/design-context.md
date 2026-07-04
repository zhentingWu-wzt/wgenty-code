# Comet Design Handoff

- Change: split-api-and-subagent-result-relief
- Phase: design
- Mode: compact
- Context hash: a8f509aa5b89d9a14b58c6de19afc0b3ffd859c23231f1e131cbac63337de250

Generated-by: comet-handoff.sh

OpenSpec remains the canonical capability spec. This handoff is a deterministic, source-traceable context pack, not an agent-authored summary.

## openspec/changes/split-api-and-subagent-result-relief/proposal.md

- Source: openspec/changes/split-api-and-subagent-result-relief/proposal.md
- Lines: 1-29
- SHA256: 0c5c8c329194567aa45f95673b48aa92d43582e0ccdf42d178517896d0e8907a

```md
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
```

## openspec/changes/split-api-and-subagent-result-relief/design.md

- Source: openspec/changes/split-api-and-subagent-result-relief/design.md
- Lines: 1-57
- SHA256: 13a8839854bf680d6c65b475ef2481636cbd0954e7eef5710d079c77b26ca484

```md
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
```

## openspec/changes/split-api-and-subagent-result-relief/tasks.md

- Source: openspec/changes/split-api-and-subagent-result-relief/tasks.md
- Lines: 1-16
- SHA256: 2f78f8085fcc3030e787ea1aa1dbc7df7ff9a61b83ecd96b765d8758dbaafddc

```md
## 1. src/api 模块拆分(纯重构,代码已在工作区)

- [ ] 1.1 创建 `src/api/error.rs`,移动 `wrap_network_error` / `format_api_error` 及其单元测试
- [ ] 1.2 创建 `src/api/types.rs`,移动 wire-format 类型(`ToolDefinition`/`ChatMessage`/`ChatRequest`/`ChatResponse`/`StreamChunk` 等)及其单元测试
- [ ] 1.3 更新 `src/api/mod.rs`:`pub mod error/types` + `pub use types::*` + `pub(crate) use error::*`,保持 `crate::api::*` 旧路径
- [ ] 1.4 `cargo build && cargo test --lib && cargo clippy --lib` 验证行为零变化、旧 `crate::api::*` 引用仍编译

## 2. subagent 大结果交付重新设计(行为变更)

- [ ] 2.1 comet-design 阶段 brainstorming 选定缓解方案(按需加载 B1 / compaction 降级 B2 / 混合 B3)
- [ ] 2.2 查清 `to_compact()` 是否 dead code、是否应接入 compaction 作为降级入口
- [ ] 2.3 按 design 定案调整 `src/teams/subagent_mailbox.rs`:`SubagentResponse` 变体、`to_content()`、`to_compact()`、`offload_if_large()`
- [ ] 2.4 同步 `src/tools/meta/task.rs` 调用方(若 `to_content` 返回语义变化)
- [ ] 2.5 若选 B2:扩展 `src/tui/agent/compaction.rs` 识别上下文中的大 subagent 结果并替换为磁盘引用
- [ ] 2.6 更新/新增测试覆盖选定方案的 spec scenarios(不丢细节 + token 可控 + 磁盘恢复 + 持久化失败降级)
- [ ] 2.7 `cargo test` 验证 `subagent-result-delivery` spec 全部 scenarios 通过
```

## openspec/changes/split-api-and-subagent-result-relief/specs/subagent-result-delivery/spec.md

- Source: openspec/changes/split-api-and-subagent-result-relief/specs/subagent-result-delivery/spec.md
- Lines: 1-35
- SHA256: 0806d011ee2e6c8b7943c2efc6727beb7ababffe3dafdae44ab89c13cbf6f7db

```md
## ADDED Requirements

### Requirement: Large subagent results remain accessible without loss
When a subagent produces a result exceeding the persistence threshold, the system SHALL preserve the full content such that the parent agent can access it without lossy truncation. The parent agent SHALL NOT be presented with a fixed-length prefix summary as the only representation of a large result.

#### Scenario: Parent agent can recover full content of a large result
- **WHEN** a subagent produces a result larger than `MAX_INLINE_RESULT_LEN` (4000 chars)
- **THEN** the full content SHALL be persisted to disk
- **AND** the parent agent SHALL be able to access the full content (either inline or via a recovery path)

#### Scenario: Large result not replaced by short prefix-only summary
- **WHEN** a subagent result exceeds the persistence threshold
- **THEN** the parent agent SHALL NOT receive only a 200-character prefix summary as the sole representation
- **AND** the full content SHALL remain recoverable

### Requirement: Large result delivery controls parent context token cost
The system SHALL deliver large subagent results to the parent agent through a mechanism that bounds the parent agent's context token consumption, rather than unconditionally inlining the full content. The specific mechanism (on-demand loading, compaction-time degradation, or hybrid) is determined by design.

#### Scenario: Full content not unconditionally inlined
- **WHEN** a subagent result exceeds the persistence threshold
- **THEN** the system SHALL NOT unconditionally inline the entire content into the parent agent's context as the sole delivery strategy
- **AND** a token-bounding mechanism SHALL be in place

### Requirement: Disk persistence for recovery
When a subagent result exceeds the persistence threshold, the system SHALL persist a copy to the JSONL mailbox so the full content can be recovered later (e.g., after context compaction).

#### Scenario: Large result persisted to disk
- **WHEN** a subagent result exceeds `MAX_INLINE_RESULT_LEN`
- **THEN** a copy SHALL be written to the JSONL mailbox file
- **AND** the recovery path SHALL be communicated to the parent agent

#### Scenario: Persistence failure does not lose content
- **WHEN** disk persistence fails for a large result
- **THEN** the full content SHALL still be returned to the parent agent inline (no truncation)
- **AND** the failure SHALL be logged
```

