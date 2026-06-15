# Comet Design Handoff

- Change: codegraph-query-and-explainability
- Phase: design
- Mode: compact
- Context hash: 534ca57884c765318e077225daf13bbb9ba53eb2c2ba88bf684c911cafc169a2

Generated-by: comet-handoff.sh

OpenSpec remains the canonical capability spec. This handoff is a deterministic, source-traceable context pack, not an agent-authored summary.

## openspec/changes/codegraph-query-and-explainability/proposal.md

- Source: openspec/changes/codegraph-query-and-explainability/proposal.md
- Lines: 1-52
- SHA256: e69b9b8831fe11b3e94e1e8546fce69b4b2b3dae0120de0bcb4766ea41a41f20

```md
## Why

#1 codegraph-agent-adoption 已通过 prompt + tool description + error message 三层修复，让 Agent 在代码导航任务中主动选用 codegraph 替代 grep。但 Agent 即使选择 codegraph，当前的查询结果仍缺乏可信度和可追溯性：没有置信度标注（Agent 无法判断结果可靠性）、没有调用路径证据（Agent 只能看到一跳 callers/callees）、没有审计日志（无法复现和调试查询）。同时查询入口仅 codegraph_node 和 codegraph_explore 两种，缺少批量查询、路径搜索、模块概要等常用模式。

本 change 从查询能力增强和可解释性两个维度改进 codegraph，让 Agent 不仅能找到符号、还能理解为什么找到、结果有多可靠、以及如何复现。

## What Changes

- **可解释性 — 调用路径树**：`codegraph_explore` 输出格式从扁平列表改为多跳调用路径树（深度 ≤5），每跳显示证据（file:line + RelKind），让 Agent 能追踪完整调用链
- **可解释性 — 审计日志**：新增 `.codegraph/audit.log`（结构化 JSONL，append-only），记录每次查询的时间/类型/参数/结果摘要/audit_id。Agent 和用户可通过 jq 复现查询
- **可解释性 — 置信度与来源**：`codegraph_node` 和 `codegraph_explore` 输出增加 `confidence` 字段（High=treesitter-AST 直接解析，Medium=文本匹配，Low=推断，Unresolved=找不到）和 `source` 字段，利用已有 Confidence 枚举
- **查询增强 — 模糊匹配**：`codegraph_node` 在精确匹配无结果时自动 Levenshtein ≤3 模糊补全（与现有 spec「Symbol not found → suggestions」场景一致）
- **查询增强 — 过滤排序**：输出支持按 confidence/relevance 排序和按 name/kind 过滤
- **新查询模式 — call_path**：新增两点间最短调用路径查询（符号 A → B 路径，Dijkstra 图搜索）
- **新查询模式 — symbol_batch**：一次查询多个符号名，聚合结果
- **新查询模式 — module_summary**：模块概要（符号列表、导出函数、依赖关系）
- **现有能力修改**：`symbol-query`（增加 confidence/source/fuzzy）、`call-graph`（增加 call_path/path_tree）

## Capabilities

### New Capabilities

- `codegraph-explainability`：codegraph 查询结果的可解释性保障，包括结构化审计日志（JSONL，可复现）、调用路径树的逐跳证据标注、输出中 confidence/source 字段。该 capability 规定审计日志的格式约束、路径树深度和证据要求、confidence 的分级来源策略。
- `codegraph-advanced-query`：codegraph 的增强查询模式，包括 call_path（两点间最短路径）、symbol_batch（批量查询）、module_summary（模块概要），以及模糊匹配、过滤排序。该 capability 规定新查询模式的输入输出格式、模糊匹配的算法约束（Levenshtein ≤3）、过滤排序的支持字段。

### Modified Capabilities

- `symbol-query`：`codegraph_node` 输出增加 `confidence` 和 `source` 字段；增加 fuzzy 匹配行为（精确未命中 → Levenshtein ≤3 建议）；增加 `sort_by` 和 `filter` 参数
- `call-graph`：`codegraph_explore` 输出格式从扁平列表改为多跳路径树（深度 ≤5）；增加 `call_path` 查询模式（两点间最短路径）；增加逐跳证据标注（file:line + RelKind）

## Impact

- **修改文件**：
  - `src/tools/codegraph/query.rs`：调用路径树算法 + 模糊匹配 + 过滤排序 + 新查询模式
  - `src/tools/codegraph/types.rs`：可能新增 CallPath / ModuleSummary 类型；现有 Confidence 枚举增强
  - `src/tools/codegraph/store.rs`：可能新增图遍历查询（Dijkstra）所需的 SQL 查询
  - `src/tools/codegraph/tools.rs`：新增 call_path / symbol_batch / module_summary 三个工具；修改 codegraph_node / codegraph_explore 的 input/output schema
  - `src/tools/codegraph/indexer.rs`：可能需要增强索引时的置信度标记（tree-sitter 解析源记录）
  - 新增 `.codegraph/audit.log`：结构化审计日志
  - `openspec/specs/symbol-query/spec.md`、`openspec/specs/call-graph/spec.md`：修改场景
- **新增文件**：
  - `src/tools/codegraph/audit.rs`：审计日志写入模块
  - `src/tools/codegraph/call_path.rs`：两点间路径搜索
- **不修改**：`src/prompts/`（#1 已完成）；TUI 显示；MCP 协议层；索引引擎核心
- **新增依赖**：可能引入 `levenshtein` crate（模糊匹配）；审计日志无外部依赖（纯 `std::fs` append）
- **运行影响**：审计日志自然增长（每次查询追加 ~200 字节）；图搜索（call_path）首次查询可能较慢（需全量加载关系），后续缓存
- **验收数据来源**：bench-agent-replay.sh（#1 产物）重跑验证 codegraph 调用率不降；#0 基线报告中性能数据作为对比基线
- **下游依赖**：`codegraph-multilang-and-deep-graph` (#3) 在扩展多语言索引时，会依赖本 change 的调用路径树和审计日志基础；新查询模式也会在多语言场景中直接获益
- **风险**：中
  - 调用路径树图搜索可能引入 O(n²) 复杂度（缓解：深度限制 ≤5、首次缓存、最大节点限制）
  - 审计日志文件大小自然增长（缓解：JSONL 压缩性高、可设置 rotate 上限，归档旧 log）
  - 新增 3 个 tool 可能让 Agent 工具选择更复杂（缓解：#1 的 playbook 已提供优先级指导）
```

## openspec/changes/codegraph-query-and-explainability/design.md

- Source: openspec/changes/codegraph-query-and-explainability/design.md
- Lines: 1-146
- SHA256: db60a2ddc00da7920fd7d2cc8aa3e64c4b7336712a8799c6b2277470d8e4e7f1

[TRUNCATED]

```md
## Context

#0 和 #1 已完成。当前 codegraph 的查询能力仅限于两种模式：
- `codegraph_node(symbol)`：精确匹配单符号，返回定义/签名/引用/callers-calers
- `codegraph_explore(query)`：模糊探索，返回相关符号和关系

两者输出均缺乏 confidence/source 字段、无审计追踪、无可解释的调用路径证据。Agent 即使选择了 codegraph，也无法判断结果可靠性或追溯查询过程。用户原始需求中的"可解释性 = 调用路径树 + 审计日志"在本 change 中实现。

现状代码关键事实：
- `query.rs` 已有 QueryEngine（node/exlore 方法）
- `types.rs` 已有 Confidence 枚举（High/Medium/Low/Unresolved）但实际产出薄弱
- `store.rs` 已有 IndexStore（SQLite 读写）
- 关系仅 4 种 RelKind：calls/implements/contains/imports
- 索引持久化在 `.codegraph/index.db`（由 indexer.rs 生成）

约束：不修改索引引擎核心（tree-sitter 解析链）——归 #3；不修改 MCP 层、不修改 TUI。

## Goals / Non-Goals

**Goals:**

- 实现调用路径树输出（深度 ≤5，每跳带 file:line + RelKind 证据）
- 新增 `.codegraph/audit.log`（结构化 JSONL，append-only）
- 在 codegraph_node 和 codegraph_explore 输出中填充 confidence（基于解析源分级）和 source 字段
- 实现模糊匹配（Levenshtein ≤3）
- 实现输出过滤排序（按 confidence/relevance + name/kind）
- 新增 3 种查询模式：call_path、symbol_batch、module_summary
- 在 #1 的 bench-agent-replay.sh 上重新验证 codegraph 调用率不降

**Non-Goals:**

- 不修改 tree-sitter 解析链（索引如何产生 confidence——只需要标记"此符号来自 tree-sitter AST 解析"标志位，归 #3）
- 不引入 NLP/LLM 语义检索（超出当前范围）
- 不修改 codegraph index 的数据 schema（只读不改写）
- 不在 TUI 中显示审计日志（仅文件级）
- 不扩展现有关系类型（归 #3）

## Decisions

### D1：调用路径树算法 —— BFS/DFS + 深度限制

**决策**：从目标符号出发做 BFS（宽度优先）遍历 calls 关系，深度 ≤5 截断。每跳记录 {from_symbol, to_symbol, rel_kind, file:line}。

**数据结构**：
```json
{
  "symbol": "run_async",
  "call_paths": [
    {
      "depth": 3,
      "hops": [
        {"from": "main", "to": "init", "rel": "calls", "location": "src/cli/mod.rs:45"},
        {"from": "init", "to": "setup_agent", "rel": "calls", "location": "src/agent/core.rs:120"},
        {"from": "setup_agent", "to": "run_async", "rel": "calls", "location": "src/agent/core.rs:201"}
      ]
    }
  ]
}
```

**理由**：
- BFS 天然支持最短路径，与 spec「最多 5 跳」且「优先展示短路径」
- 现有 calls 关系存储在 SQLite，可通过递归 CTE 或内存 graph 遍历
- 深度限制防止指数爆炸

**替代方案**：
- DFS —— 拒绝。单路径可能过长，跳过更短的备选路径
- 不缓存图（每次全量 SQL 查）—— 拒绝。小项目 OK，大项目 O(n²)

### D2：审计日志格式 —— JSONL append + audit_id

**格式**（每行一个 JSON 对象）：
```json
{"ts":"2026-06-15T10:30:00Z","audit_id":"550e8400-e29b-41d4-a716-446655440000","query_type":"codegraph_node","params":{"symbol":"ToolRegistry"},"result_count":1,"elapsed_ms":12,"source_files":["src/tools/mod.rs"]}
```

**audit_id 生成**：UUID v4，每次查询生成，返回给 Agent 的 output 中包含 `audit_id` 字段。Agent 可引用 audit_id 报告异常。

**写入方式**：
- `std::fs::OpenOptions::append(true).open(log_path)` 每次查询追加
```

Full source: openspec/changes/codegraph-query-and-explainability/design.md

## openspec/changes/codegraph-query-and-explainability/tasks.md

- Source: openspec/changes/codegraph-query-and-explainability/tasks.md
- Lines: 1-83
- SHA256: 9ffc660261accd5541d7ed1f1ac6869ce96fa7fdcdcffb67aab2974661ad6d11

[TRUNCATED]

```md
# Tasks — codegraph-query-and-explainability

> 每完成一个 task 必须立即勾选并 git commit；message 体现设计意图。

## 1. 审计日志基础设施

- [ ] 1.1 新增 `src/tools/codegraph/audit.rs`：实现 AuditLogger 结构体 + `log_query()` 方法（JSONL append + audit_id 生成）
- [ ] 1.2 在 QueryEngine 中注入 AuditLogger，所有查询路径（node/explore/call_path/symbol_batch/module_summary）调用后写入日志
- [ ] 1.3 在 codegraph_node/explore 输出 JSON 中追加 `audit_id` 字段
- [ ] 1.4 commit：`feat(codegraph): audit logger — JSONL append-only with UUID audit_id`

## 2. Confidence + Source 标注

- [ ] 2.1 修改 `src/tools/codegraph/types.rs`：Confidence 枚举增加 `source` 字段和映射函数 `from_parse_source()`
- [ ] 2.2 修改 QueryEngine 的 node/explore 方法：填充返回结果中每个 symbol 的 `confidence` 和 `source`
- [ ] 2.3 修改 codegraph_node/explore 的 output JSON schema：增加 `confidence`（"high"/"medium"/"low"/"unresolved"）和 `source` 字段
- [ ] 2.4 commit：`feat(codegraph): confidence + source annotation on query results`

## 3. 调用路径树

- [ ] 3.1 新增 `src/tools/codegraph/call_path.rs`：实现 graph 构建（从 IndexStore 加载全量 calls）+ BFS 遍历（深度 ≤5，记录 file:line + RelKind）
- [ ] 3.2 修改 codegraph_explore 输出格式：当查询涉及 calls 关系时，附加 `call_paths` 字段（路径树 JSON）
- [ ] 3.3 commit：`feat(codegraph): multi-hop call path tree with evidence (depth ≤5)`

## 4. 模糊匹配

- [ ] 4.1 实现 `levenshtein_distance()` 函数（≤ `src/tools/codegraph/query.rs` 或独立 `fuzzy.rs`）
- [ ] 4.2 修改 codegraph_node：精确匹配无结果时，从索引中取候选，按距离排序返回 top 5（≤3 跳 + length_diff ≤50%）
- [ ] 4.3 编写单元测试覆盖 Unicode/空字符串/极长符号
- [ ] 4.4 commit：`feat(codegraph): fuzzy symbol matching with Levenshtein ≤3`

## 5. 过滤与排序

- [ ] 5.1 修改 codegraph_node/explore 参数 schema：增加 `sort_by`（confidence/name）和 `filter`（kind/name_prefix）
- [ ] 5.2 实现排序逻辑（confidence 降序 + name 升序）和过滤逻辑
- [ ] 5.3 commit：`feat(codegraph): filter and sort support for query results`

## 6. 新查询模式 — call_path

- [ ] 6.1 新增 `CallPathTool` struct（实现 Tool trait）：`from` + `to` symbol 输入，输出最短路径 hops[]
- [ ] 6.2 实现 Dijkstra 最短路径搜索（基于 memory graph from call_path.rs）
- [ ] 6.3 无路径场景输出 `{"path_found": false, "reason": "..."}`
- [ ] 6.4 commit：`feat(codegraph): call_path tool — shortest path between two symbols`

## 7. 新查询模式 — symbol_batch

- [ ] 7.1 新增 `SymbolBatchTool` struct：输入 `symbols[]`（max 10），输出每个 symbol 的 node 结果数组
- [ ] 7.2 聚合逻辑复用 QueryEngine::codegraph_node()
- [ ] 7.3 commit：`feat(codegraph): symbol_batch tool — batch symbol lookup`

## 8. 新查询模式 — module_summary

- [ ] 8.1 新增 `ModuleSummaryTool` struct：输入 `module_path`，输出该模块下所有符号列表 + 导出函数 + 依赖关系
- [ ] 8.2 实现基于 IndexStore 的模块过滤查询（SQLite `WHERE file_path LIKE 'module_path%'`）
- [ ] 8.3 commit：`feat(codegraph): module_summary tool — module-level overview`

## 9. Tool 注册与集��

- [ ] 9.1 在 tool registry 中注册 call_path / symbol_batch / module_summary 三个新 tool
- [ ] 9.2 修改所有 5 个 codegraph tool 的 `description()` 为新格式（PREFER FOR/AVOID WHEN，与 #1 保持一致）
- [ ] 9.3 commit：`feat(codegraph): register call_path/symbol_batch/module_summary tools`

## 10. Spec 同步

- [ ] 10.1 创建 delta specs：`specs/codegraph-explainability/spec.md`（审计日志 + confidence + 调用路径树证据）
- [ ] 10.2 创建 delta specs：`specs/codegraph-advanced-query/spec.md`（call_path + symbol_batch + module_summary + fuzzy + filter-sort）
- [ ] 10.3 修改 delta specs：`specs/symbol-query/spec.md`（confidence/source + fuzzy）
- [ ] 10.4 修改 delta specs：`specs/call-graph/spec.md`（call_path + 路径树）

## 11. 验收与回归

- [ ] 11.1 跑 cargo build + cargo test 无回归
- [ ] 11.2 跑 bench-agent-replay.sh 验证 codegraph 调用率不降（≥ #1 存档水平）
- [ ] 11.3 手动验证 audit.log 文件正常写入（jq 可读）
- [ ] 11.4 手动验证 call_path 两点间路径（选取 3 对已知调用关系的符号）
- [ ] 11.5 手动验证 fuzzy 匹配（输入近似符号名观察 top 5 建议）
- [ ] 11.6 手动验证 symbol_batch 批量和 module_summary 输出正确

## 12. 验证与归档

```

Full source: openspec/changes/codegraph-query-and-explainability/tasks.md

## openspec/changes/codegraph-query-and-explainability/specs/call-graph/spec.md

- Source: openspec/changes/codegraph-query-and-explainability/specs/call-graph/spec.md
- Lines: 1-67
- SHA256: 28c412b8af3c61f8bab915c2b64839b1d140a8dc83f2ba9931745636775425c7

```md
## MODIFIED Requirements

### Requirement: Caller analysis

The system SHALL return the list of all functions that call a given function. The `codegraph_explore` tool description SHALL include scenario-based usage guidance ("PREFER FOR ... AVOID WHEN ...").

#### Scenario: Direct callers

- **WHEN** querying `codegraph_node("execute")` with `callers` option
- **THEN** the system returns every function that directly invokes `execute()`, with call site location

#### Scenario: No callers (entry point)

- **WHEN** querying callers for `main()`
- **THEN** the system returns an empty caller list with `is_entry_point: true` indication

#### Scenario: Tool description includes scenario guidance

- **WHEN** Agent reads the `codegraph_explore` tool description
- **THEN** the description includes "PREFER FOR" and "AVOID WHEN" clauses (consistent with #1 agent-adoption)

### Requirement: Callee analysis

The system SHALL return the list of all functions called by a given function.

#### Scenario: Direct callees

- **WHEN** querying `codegraph_node("run_async")` with `callees` option
- **THEN** the system returns every function directly called within `run_async()`, with call site locations

#### Scenario: Leaf function

- **WHEN** a function makes no calls to other user-defined functions
- **THEN** the system returns an empty callee list with `is_leaf: true` indication

### Requirement: Multi-hop call path tree

The system SHALL support querying call relationships as a multi-hop path tree from a given symbol, up to depth 5, with per-hop evidence.

#### Scenario: Build path tree from symbol

- **WHEN** `codegraph_explore` results include call relationships
- **THEN** the output includes a `call_paths` field containing an array of paths, each with a `hops[]` array where each hop contains `from` (symbol name), `to` (symbol name), `rel` (RelKind), `location` (file:line)

#### Scenario: Depth truncation

- **WHEN** the call path tree exceeds 5 levels of depth
- **THEN** paths are truncated at depth 5 and the response includes `truncated: true`

#### Scenario: Per-hop evidence

- **WHEN** any hop in the call path tree is rendered
- **THEN** the hop MUST include the `location` field (file path and line number) and `rel` field (which RelKind connects the two symbols)

### Requirement: Two-point shortest call path

The system SHALL provide a `call_path` tool that finds the shortest call path between two named symbols.

#### Scenario: Path found

- **WHEN** querying `call_path("main", "run_async")` and a call path exists
- **THEN** the system returns the shortest path as a hops[] array with total depth and per-hop evidence (from/to/rel/location)

#### Scenario: No path

- **WHEN** the two symbols have no connecting call path in the index
- **THEN** the system returns `{"path_found": false, "reason": "no_connecting_path"}`
```

## openspec/changes/codegraph-query-and-explainability/specs/codegraph-advanced-query/spec.md

- Source: openspec/changes/codegraph-query-and-explainability/specs/codegraph-advanced-query/spec.md
- Lines: 1-71
- SHA256: e51dedecb889d8af377acbba996e87d2ed34ef3a6bfd39efa04e72e6ea7b0f5c

```md
## ADDED Requirements

### Requirement: call_path 两点间路径查询

系统 SHALL 提供 `call_path` 工具，查询从符号 A 到符号 B 的最短调用路径。

#### Scenario: 路径存在

- **WHEN** 查询 `call_path("main", "run_async")` 且调用路径存在
- **THEN** 返回一条 hops[] 数组，每跳包含 from/to/RelKind/file:line，加上 `depth` 总跳数

#### Scenario: 路径不存在

- **WHEN** 查询两个符号之间没有连接路径
- **THEN** 返回 `{"path_found": false, "reason": "no_connecting_path"}` 而非空数组

### Requirement: symbol_batch 批量查询

系统 SHALL 提供 `symbol_batch` 工具，一次查询多个符号名并聚合结果。

#### Scenario: 批量查询

- **WHEN** 查询 `symbol_batch(["ToolRegistry", "StreamEvent", "run_async"])`（最多 10 个）
- **THEN** 返回一个数组，每个元素等同于该符号的 `codegraph_node` 查询结果（定义位置、签名、callers/callees、confidence/source）

#### Scenario: 超量限制

- **WHEN** 查询的 symbols 数组超过 10 个
- **THEN** 返回错误 "Batch size exceeds maximum (10)"

### Requirement: module_summary 模块概要

系统 SHALL 提供 `module_summary` 工具，输出指定模块路径下的符号列表、导出函数和依赖关系。

#### Scenario: 模块概要

- **WHEN** 查询 `module_summary("src/tools/codegraph")`
- **THEN** 返回该目录下所有 Rust 文件中定义的符号列表（按 SymbolKind 分组）、公开导出函数清单、以及该模块依赖的其他模块列表

#### Scenario: 模块不存在

- **WHEN** 指定的模块路径在索引中没有文件
- **THEN** 返回 `{"found": false, "reason": "no indexed files under module_path"}`

### Requirement: 模糊匹配

系统 SHALL 在 `codegraph_node` 精确匹配无结果时提供 Levenshtein 距离 ≤3 的候选建议。

#### Scenario: 精确未命中 → 模糊补全

- **WHEN** 查询的符号名在索引中不存在（精确匹配）
- **THEN** 系统自动进行模糊匹配，按 Levenshtein 距离排序返回 top 5 候选（距离 ≤3 且长度差 ≤50%）

#### Scenario: 无任何候选

- **WHEN** 精确匹配和模糊匹配均无结果
- **THEN** 返回 `not_found` 结果（与现有 spec 保持一致）

### Requirement: 过滤与排序

系统 SHALL 支持 codegraph_node / codegraph_explore 输出结果的过滤和排序。

#### Scenario: 按置信度排序

- **WHEN** `codegraph_node` 调用指定 `sort_by: "confidence"`
- **THEN** 返回结果按 confidence 降序排列（high > medium > low > unresolved）

#### Scenario: 按名称过滤

- **WHEN** `codegraph_node` 调用指定 `filter: {"name_prefix": "run_"}`
- **THEN** 仅返回名称以 "run_" 开头的符号
```

## openspec/changes/codegraph-query-and-explainability/specs/codegraph-explainability/spec.md

- Source: openspec/changes/codegraph-query-and-explainability/specs/codegraph-explainability/spec.md
- Lines: 1-58
- SHA256: e6d05d448494aba1e5a8615b5832a310a620cfb693faa500a0c729ad812a9b85

```md
## ADDED Requirements

### Requirement: 审计日志记录

系统 SHALL 在每次 codegraph 查询（codegraph_node / codegraph_explore / call_path / symbol_batch / module_summary）执行后，向 `.codegraph/audit.log` 追加一条结构化记录。

#### Scenario: 查询后写入日志

- **WHEN** 任一 codegraph 查询完成（成功或失败）
- **THEN** `.codegraph/audit.log` 中追加一条 JSONL 记录，包含：`ts`（ISO8601 UTC）、`audit_id`（UUID v4）、`query_type`、`params`、`result_count`、`elapsed_ms`、`source_files[]`

#### Scenario: 日志文件首次创建

- **WHEN** `.codegraph/audit.log` 不存在且首次查询开始
- **THEN** 系统自动创建文件并以 append-only 模式写入

#### Scenario: audit_id 可追溯

- **WHEN** Agent 收到 codegraph 查询结果
- **THEN** 结果中包含 `audit_id` 字段，Agent 可通过 jq 或 file_read 在 `.codegraph/audit.log` 中按 audit_id 检索该条查询

#### Scenario: 并发写入安全

- **WHEN** 多个查询并发执行
- **THEN** 每条查询的日志记录完整写入（不交叉、不截断），OS 行级写入原子性保证

### Requirement: 调用路径树证据

系统 SHALL 在 call graph 查询结果中提供多跳调用路径，每跳标注来源文件和行号。

#### Scenario: 多跳路径展示

- **WHEN** `codegraph_explore` 返回涉及 calls 关系的结果
- **THEN** 结果包含 `call_paths` 字段，每一跳包含：`from`（符号名）、`to`（符号名）、`rel`（RelKind）、`location`（file:line）

#### Scenario: 深度限制

- **WHEN** 调用路径深度超过 5 跳
- **THEN** 路径在第 5 跳截断，并在结果中标注 `truncated: true`

### Requirement: 置信度与来源标注

系统 SHALL 在所有 codegraph 查询结果中为每个 symbol 标注 confidence 和 source 字段。

#### Scenario: tree-sitter 直接解析

- **WHEN** symbol 由 tree-sitter AST 直接解析产生（现有索引的默认来源）
- **THEN** `confidence` = "high"，`source` = "treesitter-ast"

#### Scenario: 间接推断

- **WHEN** symbol 由非直接 AST 节点的推断产生（如 call_path 的多跳关系链）
- **THEN** `confidence` = "medium"，`source` = "inferred"

#### Scenario: 模糊匹配

- **WHEN** symbol 由模糊匹配（Levenshtein ≤3）补全提供
- **THEN** `confidence` = "low"，`source` = "fuzzy-match"
```

## openspec/changes/codegraph-query-and-explainability/specs/symbol-query/spec.md

- Source: openspec/changes/codegraph-query-and-explainability/specs/symbol-query/spec.md
- Lines: 1-49
- SHA256: 7a14234b66294c6bb0aaa82783a13212b32d81acc27e709214ad1f7bcbb84fbf

```md
## MODIFIED Requirements

### Requirement: Symbol definition lookup

The system SHALL return the exact file path, line number, column, signature, and visibility of a symbol given its name. The result SHALL include `confidence` and `source` fields for explainability. The query SHALL support fuzzy matching when exact match fails. The `codegraph_node` tool description SHALL include scenario-based usage guidance ("PREFER FOR ... AVOID WHEN ...").

#### Scenario: Find a function definition

- **WHEN** querying `codegraph_node("ToolRegistry")`
- **THEN** the system returns `src/tools/mod.rs:75` with the full signature, visibility, and `confidence: "high"`, `source: "treesitter-ast"`

#### Scenario: Find a struct definition

- **WHEN** querying `codegraph_node("StreamEvent")`
- **THEN** the system returns the file path, line, column, and all fields of the struct, with confidence/source fields

#### Scenario: Symbol not found → fuzzy suggestions

- **WHEN** querying a symbol name that does not exist in the index (exact match fails)
- **THEN** the system returns a `not_found` result with up to 5 similarly-named symbols (Levenshtein distance ≤ 3, length difference ≤ 50%), each with `confidence: "low"` and `source: "fuzzy-match"`

#### Scenario: Symbol not found → no fuzzy candidates

- **WHEN** neither exact nor fuzzy matching yields results
- **THEN** the system returns `not_found` with an empty suggestions array

#### Scenario: Tool description includes scenario guidance

- **WHEN** Agent reads the `codegraph_node` tool description
- **THEN** the description includes "PREFER FOR" clause and "AVOID WHEN" clause (consistent with #1 agent-adoption)

#### Scenario: Filter and sort support

- **WHEN** querying `codegraph_node` with `sort_by: "confidence"` and/or `filter: {"name_prefix": "run_"}`
- **THEN** results are sorted by confidence descending and filtered to matching names only

### Requirement: Explainability fields in output

The system SHALL include `audit_id`, `confidence`, and `source` fields in every codegraph query response.

#### Scenario: audit_id present

- **WHEN** any codegraph query returns results
- **THEN** the response includes an `audit_id` field (UUID v4) that matches an entry in `.codegraph/audit.log`

#### Scenario: confidence and source per symbol

- **WHEN** `codegraph_node` returns one or more symbols
- **THEN** each symbol in the result has `confidence` ("high"/"medium"/"low"/"unresolved") and `source` ("treesitter-ast"/"regex-match"/"inferred"/"fuzzy-match"/"none")
```

