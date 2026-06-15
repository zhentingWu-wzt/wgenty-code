# Comet Design Handoff

- Change: codegraph-agent-adoption
- Phase: design
- Mode: compact
- Context hash: 1094ed7d8a9fb986a9eb232694bb9db2bf87aad019d4a3e6eaec12a1ed717b0b

Generated-by: comet-handoff.sh

OpenSpec remains the canonical capability spec. This handoff is a deterministic, source-traceable context pack, not an agent-authored summary.

## openspec/changes/codegraph-agent-adoption/proposal.md

- Source: openspec/changes/codegraph-agent-adoption/proposal.md
- Lines: 1-52
- SHA256: 39300a82f1d0e3f8b0973b39f8ec548c8bdf3b56ef8090aba4a4d0d9eb6677ee

```md
## Why

#0 codegraph-baseline-spike 已建立量化基线：codegraph 工具调用率仅 0.05%（1/1959），session 采纳率 1.4%（1/71），codegraph_explore 从未被使用。根因分析（`scripts/codegraph-bench/root-cause-analysis.md`）确定 top 3 根因：

1. **System Prompt 中 grep 被列为代码搜索首选**（`src/prompts/base.md:117-119, 141-153`），codegraph 工具完全不在工具列表和「When to use each tool」对比表中。Agent 不知道 codegraph 存在
2. **工具描述缺乏场景引导**（`src/tools/codegraph/tools.rs:61-63, 175-177`），description 是功能导向（"what it does"），没有"何时优先用"的对比性引导
3. **Lazy-init 反馈缺失**（`src/tools/codegraph/tools.rs:10-34`），初始化无成功信号、错误信息无操作建议

本 change 针对这三个根因从 prompt 层、工具描述层、错误反馈层同步修复，让 Agent 在合适场景主动使用 codegraph 替代 grep。

## What Changes

- 调整 `src/prompts/base.md`：
  - 在「Search」工具段落加入 `codegraph_node` 和 `codegraph_explore`，排在 `grep` 之前
  - 在「When to use each tool」对比表中将「Find where a function is defined」、「Find callers of a function」、「Find implementations of a trait」等场景的推荐工具改为 codegraph，grep 降为兜底
- 调整 `src/prompts/`（位置由 design 阶段决定）：新增「代码导航 playbook」段落，明确 codegraph→grep→file_read 的标准工作流
- 修改 `src/tools/codegraph/tools.rs`：
  - 强化 `codegraph_node` description，加入 "PREFER FOR symbol definitions, callers, references"
  - 强化 `codegraph_explore` description，加入场景化引导（模块结构、调用图浏览）
  - 优化 lazy-init 错误文案：明确告知如何通过 `wgenty-code codegraph index` 修复
- 新增 `scripts/codegraph-bench/bench-agent-replay.sh`：在新 prompt 上回放 14 条标准导航任务，输出工具调用分布 + 分层统计 JSON 报告
- 在 wgenty-code 自身仓库验证分层阈值（强项类 ≥60%、其他类 ≥25%）

## Capabilities

### New Capabilities

无。本次 change 修改既有 capability 的 spec 行为，不引入新 capability。

### Modified Capabilities

- `symbol-query`：`codegraph_node` 工具的 description 字段从纯功能描述变更为含场景引导（"PREFER FOR..."）；spec 验收场景需补充「工具描述包含场景引导」要求。
- `call-graph`：`codegraph_explore` 工具的 description 字段同上；spec 需补充场景引导验收。
- `codegraph-lazy-init`：lazy-init 错误信息从泛化提示变更为明确的可操作建议（包含具体修复命令）；spec 需补充错误反馈格式要求。

## Impact

- **修改文件**：
  - `src/prompts/base.md`（codegraph 加入工具列表 + 对比表）
  - `src/prompts/` 下其他 prompt 文件（位置 design 决定，新增代码导航 playbook）
  - `src/tools/codegraph/tools.rs`（强化 description + 错误文案）
  - `openspec/specs/symbol-query/spec.md`、`openspec/specs/call-graph/spec.md`、`openspec/specs/codegraph-lazy-init/spec.md`（修改场景描述）
- **新增文件**：`scripts/codegraph-bench/bench-agent-replay.sh`（回归测试脚本）
- **不修改**：codegraph 索引引擎、查询逻辑（`src/tools/codegraph/{indexer,query,store,parser}.rs`）；TUI 显示；MCP 协议层
- **不引入新依赖**
- **运行影响**：Agent 行为发生显式变化（更多调用 codegraph_node/codegraph_explore）；对仅使用现有 grep/file_read 工作流的 session 无影响（这些工具仍可用，只是优先级降低）
- **验收数据来源**：#0 baseline 报告（`openspec/changes/archive/2026-06-15-codegraph-baseline-spike/`）和 14 条标准任务集（`scripts/codegraph-bench/agent-tasks/nav-001~014.yaml`）
- **下游 change 依赖**：`codegraph-query-and-explainability` (#2)、`codegraph-multilang-and-deep-graph` (#3) 在各自 design 阶段会引用本 change 的采纳率提升结果作为查询能力 / 多语言改进的需求驱动证据
- **风险**：中
  - prompt 修改可能影响其他类型任务的工具选择行为（缓解：S2 验收场景要求不破坏现有功能）
  - codegraph 索引未建时新错误文案需精准（缓解：错误文案修改仅文案，不改架构）
  - 14 条任务集代表性有限（缓解：分层阈值已宽松到可达，本 change 验收以任务集为准；真实使用监控留给后续 change）
```

## openspec/changes/codegraph-agent-adoption/design.md

- Source: openspec/changes/codegraph-agent-adoption/design.md
- Lines: 1-158
- SHA256: 2b12f6c4cc9153ddbf14a761481ed709052aa7db714eb0357618643b4eb044b7

[TRUNCATED]

```md
## Context

#0 codegraph-baseline-spike 的根因分析（`scripts/codegraph-bench/root-cause-analysis.md`）确定 Agent 不主动使用 codegraph 的 top 3 根因，并量化了基线（codegraph 调用率 0.05%）。本 change 是后续 3 个改进 change 中风险最低、改动最少的，作为第一个修复打头阵。

现状关键事实：
- codegraph 工具（`codegraph_node`、`codegraph_explore`）已通过 MCP 暴露
- 索引能力已就绪（`.codegraph/index.db`）
- `src/prompts/base.md:117-119` 工具列表中 grep 排第一位、codegraph 完全不在列表
- `src/prompts/base.md:141-153` 「When to use each tool」表的"查找函数定义"推荐 grep
- `src/tools/codegraph/tools.rs:61-63, 175-177` description 是纯功能描述，无场景引导
- `src/tools/codegraph/tools.rs:10-34` lazy-init 错误信息为 `"No codegraph index found. Run 'wgenty-code codegraph index' first."` —— 信息存在但未在 prompt 层规约 Agent 收到该错误时该如何处理

约束：
- 不修改 codegraph 索引引擎、查询逻辑、MCP 层
- 不修改 TUI、不修改 OnceLock 架构
- 不引入新 Cargo 依赖

## Goals / Non-Goals

**Goals:**

- 通过 prompt + tool description + 错误文案三层修复，让 Agent 在合适场景主动选用 codegraph 替代 grep
- 在 14 条标准任务集上，达到分层阈值：强项类 ≥60%、其他类 ≥25% 使用 codegraph
- 不破坏现有非代码导航工作流（grep、file_read、glob 仍正常工作）
- 提供可重跑的回归测试脚本 `bench-agent-replay.sh`，让后续每次 prompt 修改都能自测

**Non-Goals:**

- 不修改 codegraph 业务逻辑（索引、查询、解析） —— 归 #2/#3
- 不增加 TUI 状态显示 —— 避免 src/tui/ 改动
- 不修改 OnceLock 初始化架构 —— 仅文案优化
- 不强制 Agent 100% 用 codegraph —— 保留 grep/file_read/glob 作为兜底
- 不增量索引、不预热索引 —— 这些是性能优化，归 #2

## Decisions

### D1：分三层修复，每层独立验证

**决策**：将修复分为 3 个独立可验证层：
- 层 A — Prompt 层：修改 `src/prompts/base.md`（工具列表 + 对比表）
- 层 B — 工具描述层：修改 `src/tools/codegraph/tools.rs` 的 description
- 层 C — 错误反馈层：优化 lazy-init 的 ToolError 文案

**理由**：
- 三层修复对应根因 1/2/3，独立验证便于归因（哪一层贡献了多少采纳率提升）
- 每层都可单独 commit + 跑 bench-agent-replay.sh 看效果
- 失败可独立回滚

**替代方案**：
- 一次性修改三层 — 拒绝。无法归因，回滚粒度粗
- 仅做层 A — 拒绝。description 不改，Agent 即使知道 codegraph 存在也不知何时用

### D2：代码导航 playbook 放在 base.md 而非新文件

**决策**：「代码导航 playbook」作为新段落加入 `src/prompts/base.md`，不新增独立 prompt 文件。

**理由**：
- base.md 是 Agent 系统 prompt 的核心，新文件需要额外的加载机制
- playbook 内容与「Search」段落 + 「When to use each tool」表逻辑相邻
- 新增独立文件会触发架构变更，超出本 change 边界

**替代方案**：
- 放在 `src/prompts/collaboration.md`（如存在）— 待 build 阶段确认是否有该文件
- 放在新文件 `src/prompts/code-navigation.md` — 拒绝，需要修改加载逻辑

### D3：tool description 用「PREFER FOR ... AVOID WHEN ...」结构

**决策**：description 重写为以下格式：

```
codegraph_node: Look up a Rust symbol by name. Returns definition, signature, references, callers/callees.
PREFER FOR: finding symbol definitions, listing callers/callees, finding references.
AVOID WHEN: searching for text patterns or non-symbol concepts (use grep instead).
```

**理由**：
- 显式列出场景，让 Agent 通过模式匹配做决策（Anthropic / OpenAI 工具调用模型的标准对策）
- AVOID WHEN 提供反向引导，避免 codegraph 被滥用到 grep 适合的场景
- 与 prompt 中「When to use each tool」表互相印证

```

Full source: openspec/changes/codegraph-agent-adoption/design.md

## openspec/changes/codegraph-agent-adoption/tasks.md

- Source: openspec/changes/codegraph-agent-adoption/tasks.md
- Lines: 1-77
- SHA256: d5f1a102f0371250b6729f09f2b83634a81b70c640beb3b836a390f7561c0de3

```md
# Tasks — codegraph-agent-adoption

> 每完成一个 task 必须立即勾选并 git commit；message 体现设计意图（关联 change 名）。
> 三层修复对应 D1 决策：层 A (Prompt)、层 B (Tool description)、层 C (错误反馈)。

## 1. 准备与基线确认

- [ ] 1.1 阅读 #0 archive：`openspec/changes/archive/2026-06-15-codegraph-baseline-spike/` 与 `scripts/codegraph-bench/root-cause-analysis.md`，确认根因 top 3 仍准确
- [ ] 1.2 在当前 prompt 上跑一次 14 条 nav-XXX.yaml 任务（基线复测），确认 codegraph 调用率仍在 0.05% 量级；若变化大需重新评估阈值
- [ ] 1.3 确认 `src/prompts/` 目录下所有 prompt 文件列表（base.md、collaboration.md 等），决定 D2 中 playbook 的最终位置

## 2. 层 A — Prompt 修改

- [ ] 2.1 修改 `src/prompts/base.md` 「Search」工具段落：在 grep 之前加入 `codegraph_node` 和 `codegraph_explore`，附简短说明
- [ ] 2.2 修改 `src/prompts/base.md` 「When to use each tool」表：将以下场景的推荐工具改为 codegraph
  - "Find where a function is defined" → `codegraph_node`
  - "Find callers of a function" → `codegraph_node`
  - "Find implementations of a trait" → `codegraph_explore`
  - "Understand module structure" → `codegraph_explore`
  - 保留 grep 作为兜底（"if codegraph index unavailable"）
- [ ] 2.3 在 base.md（或 build 阶段确认的位置）新增「代码导航 playbook」段落：明确 codegraph→grep→file_read 标准工作流和何时切换
- [ ] 2.4 commit 层 A：`feat(prompts): prioritize codegraph over grep for code navigation`

## 3. 层 B — Tool description 修改

- [ ] 3.1 修改 `src/tools/codegraph/tools.rs` 中 `codegraph_node` 的 `description()` 函数，按 D3 格式重写：
  - 首句：保持功能描述
  - 第二句：`PREFER FOR: finding symbol definitions, listing callers/callees, finding references.`
  - 第三句：`AVOID WHEN: searching for text patterns or non-symbol concepts (use grep instead).`
- [ ] 3.2 修改 `src/tools/codegraph/tools.rs` 中 `codegraph_explore` 的 `description()` 函数，按 D3 格式重写：
  - 首句：描述探索能力（symbols + relationships）
  - 第二句：`PREFER FOR: exploring module structure, browsing call graphs across multiple symbols, understanding cross-module relationships.`
  - 第三句：`AVOID WHEN: looking up a single known symbol (use codegraph_node) or searching text patterns (use grep).`
- [ ] 3.3 commit 层 B：`feat(codegraph): add scenario-based guidance to tool descriptions`

## 4. 层 C — 错误文案优化

- [ ] 4.1 修改 `src/tools/codegraph/tools.rs:get_engine()` 中 "No codegraph index found" 的 ToolError message：按 D4 改为：
  ```
  No codegraph index found at .codegraph/index.db. To enable: run 'wgenty-code codegraph index' in this directory, then retry. Falling back to grep is acceptable for now.
  ```
- [ ] 4.2 commit 层 C：`feat(codegraph): improve lazy-init error message with actionable guidance`

## 5. Spec 同步（modified capabilities）

- [ ] 5.1 verify 阶段 OpenSpec 归档时按 delta 语义同步 modified specs 到主 spec（symbol-query / call-graph / codegraph-lazy-init）。本阶段不直接编辑主 spec。

## 6. 评测回归脚本

- [ ] 6.1 brainstorming D5 中的 repl 自动化方式（repl + expect / daemon API / 人工跑），写入 design.md 的 Open Questions 解答
- [ ] 6.2 实现 `scripts/codegraph-bench/bench-agent-replay.sh`：
  - 读取 `agent-tasks/nav-*.yaml` 列表
  - 对每条任务调用 wgenty-code（按 6.1 选定方式）
  - 等待 session 写入 `~/.wgenty-code/sessions/`
  - 调用 `bench-agent.sh --session <new>` 提取工具序列
- [ ] 6.3 扩展脚本输出按 task category 分层聚合：strong_categories（definition_lookup / reference_lookup / impl_enumeration）vs other_categories
- [ ] 6.4 输出 JSON 报告 `results/<ts>/agent-replay.json`，含每条任务工具序列 + 分层统计 + 与 #0 基线对比

## 7. 验收测试

- [ ] 7.1 在新 prompt + 新 description 下跑一次 `bench-agent-replay.sh`，记录分层数据
- [ ] 7.2 验证强项类（nav-001/002/003/004/007/008） ≥ 60% 使用 codegraph
- [ ] 7.3 验证其他类（nav-005/006/009-014） ≥ 25% 使用 codegraph
- [ ] 7.4 验证 grep/file_read/glob 仍在合适场景被调用（未"过度纠正"到 codegraph 但场景不对）
- [ ] 7.5 跑 cargo build 确认无回归；跑现有相关测试

## 8. 不破坏现有功能验证

- [ ] 8.1 抽查 3-5 条非代码导航 session（如配置修改、文件读取），确认行为不变
- [ ] 8.2 验证 codegraph 索引未建时新错误文案出现，且 Agent 能 fallback 到 grep
- [ ] 8.3 `git diff --stat` 确认改动范围符合预期（仅 src/prompts/、src/tools/codegraph/tools.rs、scripts/codegraph-bench/）

## 9. 验证与归档

- [ ] 9.1 运行 `openspec validate codegraph-agent-adoption` 校验
- [ ] 9.2 进入 `/comet-verify`，按 spec scenarios 逐项核对
- [ ] 9.3 verify 通过后进入 `/comet-archive`，归档到 `openspec/changes/archive/`，同步 modified specs 到主 spec
```

## openspec/changes/codegraph-agent-adoption/specs/call-graph/spec.md

- Source: openspec/changes/codegraph-agent-adoption/specs/call-graph/spec.md
- Lines: 1-23
- SHA256: 79436e5322698f46f4f2c7824f6dd3609a4a88423a89720fab41682eb1ccdebf

```md
## MODIFIED Requirements

### Requirement: Caller analysis

The system SHALL return the list of all functions that call a given function. The `codegraph_explore` tool description SHALL include scenario-based usage guidance ("PREFER FOR ... AVOID WHEN ...") to direct Agent decision-making toward call relationship and module structure exploration tasks.

#### Scenario: Direct callers

- **WHEN** querying `codegraph_node("execute")` with `callers` option
- **THEN** the system returns every function that directly invokes `execute()`, with call site location

#### Scenario: No callers (entry point)

- **WHEN** querying callers for `main()`
- **THEN** the system returns an empty caller list with `is_entry_point: true` indication

#### Scenario: Tool description includes scenario guidance

- **WHEN** Agent reads the `codegraph_explore` tool description
- **THEN** the description includes:
  - A "PREFER FOR" clause listing scenarios: exploring module structure, browsing call graphs across multiple symbols, understanding cross-module relationships
  - An "AVOID WHEN" clause distinguishing it from `codegraph_node` (single-symbol lookup) and from grep (text patterns)
- **THEN** the description differentiates `codegraph_explore` from `codegraph_node` in scope (multiple symbols / relationships vs single symbol)
```

## openspec/changes/codegraph-agent-adoption/specs/codegraph-lazy-init/spec.md

- Source: openspec/changes/codegraph-agent-adoption/specs/codegraph-lazy-init/spec.md
- Lines: 1-25
- SHA256: 3e13c511d71fca5940dc010173749cf8e2631b657c8627368b4dfe12555f2944

```md
## MODIFIED Requirements

### Requirement: CodeGraph tools auto-register with lazy initialization

CodeGraph tools SHALL be registered in the default `ToolRegistry` and SHALL lazily initialize the query engine from `.codegraph/index.db` on first use. When the index is absent, the error message SHALL provide actionable, specific guidance to enable Agent recovery without abandoning the codegraph workflow entirely.

#### Scenario: Index exists

- **WHEN** `.codegraph/index.db` exists in the current working directory
- **THEN** the engine SHALL be initialized on first tool call and SHALL remain cached for subsequent calls

#### Scenario: Index absent

- **WHEN** `.codegraph/index.db` does not exist
- **THEN** the tool SHALL return a `ToolError` whose message includes:
  - The expected index path (`.codegraph/index.db`)
  - The exact command to fix the issue (`wgenty-code codegraph index`)
  - An estimate of the fix cost to reduce hesitation (e.g., "typically takes <5s on a Rust project")
  - A fallback hint limiting grep / file_read to a temporary alternative for this single task only (to prevent permanent fallback)
- **THEN** the message SHALL use actionable, parseable instructions and SHALL avoid unbounded fallback language such as "acceptable" or "fall back to grep" without a time or scope qualifier

#### Scenario: Engine initialized once

- **WHEN** `codegraph_node` or `codegraph_explore` is called multiple times
- **THEN** the engine SHALL only be opened once (subsequent calls reuse the cached instance)
```

## openspec/changes/codegraph-agent-adoption/specs/symbol-query/spec.md

- Source: openspec/changes/codegraph-agent-adoption/specs/symbol-query/spec.md
- Lines: 1-28
- SHA256: cf91304ae31cf41f15550428ea2f8808dbedbf229e1a576446d913a7ca0acd22

```md
## MODIFIED Requirements

### Requirement: Symbol definition lookup

The system SHALL return the exact file path, line number, column, signature, and visibility of a symbol given its name. The `codegraph_node` tool description SHALL include scenario-based usage guidance ("PREFER FOR ... AVOID WHEN ...") to direct Agent decision-making toward symbol-related navigation tasks.

#### Scenario: Find a function definition

- **WHEN** querying `codegraph_node("ToolRegistry")`
- **THEN** the system returns `src/tools/mod.rs:75` with the full signature and visibility

#### Scenario: Find a struct definition

- **WHEN** querying `codegraph_node("StreamEvent")`
- **THEN** the system returns the file path, line, column, and all fields of the struct

#### Scenario: Symbol not found

- **WHEN** querying a symbol name that does not exist in the index
- **THEN** the system returns a `not_found` result with suggestions for similarly-named symbols (Levenshtein distance ≤ 3)

#### Scenario: Tool description includes scenario guidance

- **WHEN** Agent reads the `codegraph_node` tool description
- **THEN** the description includes:
  - A "PREFER FOR" clause listing scenarios: finding symbol definitions, listing callers/callees, finding references
  - An "AVOID WHEN" clause indicating when grep is more appropriate (text patterns, non-symbol concepts)
- **THEN** the description's first sentence describes the symbolic capability (not just "look up by name")
```

