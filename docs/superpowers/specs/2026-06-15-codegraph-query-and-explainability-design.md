---
comet_change: codegraph-query-and-explainability
role: technical-design
canonical_spec: openspec
---

# Codegraph Query & Explainability — 技术设计

> 上游 OpenSpec：`openspec/changes/codegraph-query-and-explainability/`

## 1. 概述

增强 codegraph 查询能力（调用路径树、模糊匹配、3 种新查询模式）和可解释性（审计日志、置信度标注），让 Agent 不仅能找到符号、还能理解结果的可靠性和追溯查询过程。

### OQ 确认决策

| OQ | 决策 |
|-----|------|
| 1 图缓存 | A: 内存图 (+ BFS ≤5) |
| 2 竞态 | A: Mutex\<BufWriter\> |
| 3 batch | A: max 10 |
| 4 模块边界 | A: 目录边界 |
| 5 路径展示 | A: 单条最短路径 |

## 2. 架构

```
src/tools/codegraph/
├── audit.rs          [NEW] AuditLogger — JSONL append + audit_id
├── call_path.rs      [NEW] CallGraph — 内存图构建 + BFS/Dijkstra
├── fuzzy.rs          [NEW] levenshtein_distance + top-N
├── types.rs          [MOD] Confidence::from_parse_source()
├── store.rs          [MOD] 图查询 SQL 辅助 (WHERE file_path LIKE...)
├── query.rs          [MOD] 注入 AuditLogger + confidence 填充 + fuzzy 调用
├── tools.rs          [MOD] 新增 CallPathTool/SymbolBatchTool/ModuleSummaryTool + desc 更新
│
.codegraph/
├── audit.log         [NEW] JSONL 审计日志
```

### 数据流

```
Query → AuditLogger.log_query()
      → (if call_path) CallGraph::build() → BFS/Dijkstra
      → (if node→not_found) fuzzy::search()
      → conf = Confidence::from_parse_source()
      → output + audit_id + confidence/source

.codegraph/audit.log ← append JSONL line
```

## 3. 实现细节

### 3.1 AuditLogger (audit.rs)

```rust
pub struct AuditLogger {
    writer: Arc<Mutex<BufWriter<File>>>,
}

impl AuditLogger {
    pub fn new(log_path: &Path) -> Self;
    pub fn log_query(&self, entry: AuditEntry);
    pub fn generate_audit_id() -> String; // UUID v4
}
```

JSONL 格式：
```json
{"ts":"2026-06-15T10:30:00Z","audit_id":"550e8400-...","query_type":"codegraph_node","params":{"symbol":"ToolRegistry"},"result_count":1,"elapsed_ms":12}
```

### 3.2 CallGraph (call_path.rs)

```rust
pub struct CallGraph {
    edges: HashMap<SymbolId, Vec<Edge>>,
}
pub struct Edge { from: SymbolId, to: SymbolId, rel: RelKind, location: String }

impl CallGraph {
    pub fn build(store: &IndexStore) -> Self; // 加载全量 calls
    pub fn bfs(&self, root: SymbolId, depth: usize) -> Vec<CallPath>; // 调用路径树
    pub fn shortest_path(&self, from: SymbolId, to: SymbolId) -> Option<CallPath>; // Dijkstra
}
```

### 3.3 Fuzzy (fuzzy.rs)

```rust
pub fn levenshtein(a: &str, b: &str) -> usize;
pub fn fuzzy_search(query: &str, candidates: &[String], max_dist: usize) -> Vec<ScoredMatch>;
```

### 3.4 Confidence 映射

```rust
impl Confidence {
    pub fn from_parse_source(source: ParseSource) -> Self {
        match source {
            ParseSource::TreeSitter => Confidence::High,
            ParseSource::TextMatch => Confidence::Medium,
            ParseSource::Inferred => Confidence::Low,
            ParseSource::None => Confidence::Unresolved,
        }
    }
}
```

### 3.5 新 Tool 注册

| Tool | impl 位置 | 关键方法 |
|------|----------|---------|
| `CallPathTool` | tools.rs | `execute()` → CallGraph::shortest_path() |
| `SymbolBatchTool` | tools.rs | `execute()` → 循环 QueryEngine::codegraph_node() |
| `ModuleSummaryTool` | tools.rs | `execute()` → IndexStore::summarize_module() |

## 4. 风险与权衡

| 风险 | 缓解 |
|------|------|
| 图构建 O(n) 全量加载 | 200 edges ~10KB，可接受；大项目后续优化 |
| BufWriter panic 丢失未 flush 日志 | `panic::set_hook` 中 flush；正常退出 drop flush |
| batch 10 个 query 串行慢 | 串行执行总耗时 10×15ms=150ms，可接受 |
| fuzzy 匹配短符号噪声大 | 长度差 ≤50% 过滤 + 距离 ≤3 |

## 5. 测试策略

### 单元测试（cargo test）
- `levenshtein_distance()` 覆盖率（空/Unicode/长串/相同/完全不同）
- `Confidence::from_parse_source()` 映射
- `CallGraph::build()` 用 fixture IndexStore
- `CallGraph::shortest_path()` 已知路径验证

### 集成测试
- 启动 daemon → `codegraph_node` → 验证 audit_id 在 .audit.log 中存在
- `call_path` 验证路径存在/不存在两种情况
- `symbol_batch` 验证 10 个批量 + 超限错误

### 回归测试
- bench-agent-replay.sh 重跑验证 codegraph 调用率不降
- cargo test 全绿

## 6. Spec Patch

无 — open 阶段 specs 已完整。

## 7. Migration

纯增强，向后兼容。codegraph_node/explore 原格式保持，追加字段。
