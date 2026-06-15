---
change: codegraph-query-and-explainability
design-doc: docs/superpowers/specs/2026-06-15-codegraph-query-and-explainability-design.md
base-ref: c386c1ba6413a8e7f1c85785b9fbeddfc2f6d83a
archived-with: 2026-06-15-codegraph-query-and-explainability
---

# Codegraph Query & Explainability 实施计划

**Goal:** 增强查询能力 + 可解释性：调用路径树、审计日志、置信度、模糊匹配、3 新查询模式

**Architecture:** 新增 audit.rs / call_path.rs / fuzzy.rs + 修改 types.rs / store.rs / query.rs / tools.rs

archived-with: 2026-06-15-codegraph-query-and-explainability
---

## Phase 0: 基础设施

### Task 0.1: audit.rs — 审计日志模块

- [x] 创建 `src/tools/codegraph/audit.rs`
- [x] 实现 `AuditLogger` struct (writer: Arc<Mutex<BufWriter<File>>>)
- [x] `AuditLogger::new(path)`, `log_query(entry)`, `generate_audit_id()`
- [x] `AuditEntry` struct: ts, audit_id, query_type, params, result_count, elapsed_ms
- [x] 单元测试：verify append + id uniqueness
- [x] Commit

### Task 0.2: fuzzy.rs — 模糊匹配

- [x] 创建 `src/tools/codegraph/fuzzy.rs`
- [x] `levenshtein_distance(a, b) -> usize`
- [x] `fuzzy_search(query, candidates, max_dist) -> Vec<ScoredMatch>`
- [x] 单元测试：empty/unicode/long/same/different
- [x] Commit

### Task 0.3: call_path.rs — 图构建 + 遍历

- [x] 创建 `src/tools/codegraph/call_path.rs`
- [x] `CallGraph` struct + `Edge` struct
- [x] `CallGraph::build(store)` — 从 IndexStore 加载全量 calls
- [x] `CallGraph::bfs(root, depth)` — 调用路径树
- [x] `CallGraph::shortest_path(from, to)` — Dijkstra
- [x] 单元测试：已知路径 + no_path + depth limit
- [x] Commit

archived-with: 2026-06-15-codegraph-query-and-explainability
---

## Phase 1: 集成

### Task 1.1: types.rs — Confidence 映射

- [x] 修改 `src/tools/codegraph/types.rs`
- [x] `ParseSource` enum (TreeSitter, TextMatch, Inferred, None)
- [x] `Confidence::from_parse_source()` 方法
- [x] Commit

### Task 1.2: store.rs — 模块查询

- [x] 修改 `src/tools/codegraph/store.rs`
- [x] `summarize_module(module_path)` — WHERE file_path LIKE 'path/%'
- [x] Commit

### Task 1.3: query.rs — 注入 logger + confidence + fuzzy

- [x] 修改 `src/tools/codegraph/query.rs`
- [x] QueryEngine 注入 AuditLogger
- [x] codegraph_node: 精确未命中 → fuzzy_search
- [x] codegraph_node: 填充 confidence/source
- [x] codegraph_explore: 附加 call_paths 字段
- [x] 所有 query 方法末尾调用 `logger.log_query()`
- [x] Commit

### Task 1.4: tools.rs — 新 tool 注册 + description

- [x] 新增 `CallPathTool` struct (impl Tool trait)
- [x] 新增 `SymbolBatchTool` struct
- [x] 新增 `ModuleSummaryTool` struct
- [x] 修改 codegraph_node/explore description（确认 PREFER FOR 仍存在）
- [x] 修改 codegraph_node/explore input/output schema（confidence/source/audit_id/filter/sort_by）
- [x] 在 tool registry 中注册 3 个新 tool
- [x] Commit

### Task 1.5: mod.rs — 模块导出

- [x] 修改 `src/tools/codegraph/mod.rs`
- [x] 导出 audit / call_path / fuzzy 模块
- [x] Commit

archived-with: 2026-06-15-codegraph-query-and-explainability
---

## Phase 2: 验证

### Task 2.1: cargo build + test

```bash
cargo build && cargo test -p wgenty_code
```

### Task 2.2: 手动验证
- 启动 daemon → codegraph_node → 验证 audit.log 写入 + audit_id
- call_path 验证路径存在/不存在
- module_summary 验证输出正确
- fuzzy 验证近似符号匹配

### Task 2.3: 回归
- bench-agent-replay.sh 跑一次验证 codegraph 调用率不降

archived-with: 2026-06-15-codegraph-query-and-explainability
---

## Phase 3: 勾选

- [x] 勾选 tasks.md 所有 task
- [x] cargo build && cargo test 全绿
- [x] Guard → verify → archive
