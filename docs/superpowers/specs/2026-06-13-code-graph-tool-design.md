---
comet_change: code-graph-tool
role: technical-design
canonical_spec: openspec
---

# CodeGraph 代码图谱工具 — 技术设计

## 1. 运行时架构

```
AppState
└── Arc<CodegraphEngine>  （懒初始化，首次查询时创建）
     ├── Indexer
     │   ├── parser_pool: Vec<Parser>    (num_cpus - 1)
     │   ├── extract_symbols(AST) → Vec<Symbol>
     │   ├── extract_references(AST, symbols) → Vec<Reference>
     │   └── extract_relationships(AST, symbols) → Vec<Relationship>
     ├── QueryEngine
     │   ├── codegraph_node(symbol) → Markdown
     │   ├── codegraph_explore(query) → Markdown
     │   ├── get_callers/callees(symbol, depth) → Markdown
     │   └── fuzzy_find(name) → Vec<Suggestion>
     ├── IndexStore (SQLite, WAL mode)
     │   ├── 4 tables: files, symbols, refs, relationships
     │   └── 递归 CTE for transitive call graph (depth ≤ 5)
     └── FileWatcher (notify crate)
         ├── 监听 .rs 文件变更 → 增量索引
         └── 10 分钟无查询后自动 stop

接口层（共享同一个 Arc<CodegraphEngine>）:
├── Built-in Tools: CodegraphNodeTool / CodegraphExploreTool
├── MCP Adapter: codegraph_explore / codegraph_node
└── CLI: codegraph index / query / clean
```

## 2. 数据模型

```sql
CREATE TABLE files (
    id          INTEGER PRIMARY KEY,
    path        TEXT NOT NULL UNIQUE,
    sha256      TEXT NOT NULL,
    indexed_at  TEXT NOT NULL
);

CREATE TABLE symbols (
    id              INTEGER PRIMARY KEY,
    name            TEXT NOT NULL,
    kind            TEXT NOT NULL,  -- function|struct|enum|trait|impl|type_alias|const|static|mod|macro
    file_id         INTEGER NOT NULL REFERENCES files(id),
    line            INTEGER NOT NULL,
    col             INTEGER NOT NULL,
    signature       TEXT,
    visibility      TEXT,           -- pub|pub(crate)|pub(super)|private
    parent_module   TEXT
);

CREATE TABLE refs (
    id          INTEGER PRIMARY KEY,
    symbol_id   INTEGER NOT NULL REFERENCES symbols(id),
    file_id     INTEGER NOT NULL REFERENCES files(id),
    line        INTEGER NOT NULL,
    col         INTEGER NOT NULL,
    ref_kind    TEXT NOT NULL,      -- call|type_ref|import|method_call
    context     TEXT
);

CREATE TABLE relationships (
    id          INTEGER PRIMARY KEY,
    source_id   INTEGER NOT NULL REFERENCES symbols(id),  -- caller
    target_id   INTEGER NOT NULL REFERENCES symbols(id),  -- callee
    rel_kind    TEXT NOT NULL,      -- calls|implements|contains|imports
    file_id     INTEGER NOT NULL REFERENCES files(id),
    line        INTEGER NOT NULL,
    confidence  TEXT NOT NULL DEFAULT 'high'  -- high|medium|low|unresolved
);
```

**传递闭包（递归 CTE，depth ≤ N）**：
```sql
WITH RECURSIVE tc AS (
    SELECT r.source_id, r.target_id, 1 as depth
    FROM relationships r WHERE r.target_id = ?
    UNION ALL
    SELECT r.source_id, r.target_id, tc.depth + 1
    FROM relationships r JOIN tc ON r.target_id = tc.source_id
    WHERE tc.depth < ?
)
SELECT s.name, s.signature, s.file_id, tc.depth
FROM tc JOIN symbols s ON s.id = tc.source_id;
```

## 3. 索引引擎

### 流程

```
index_project(root, mode):
  scan → hash → diff vs files table → {added, modified, deleted, unchanged}

  parallel parse (parser_pool):
    for each file in {added ∪ modified}:
      parse → AST → extract_symbols → extract_refs → extract_relationships

  batch write (single transaction):
    DELETE stale → INSERT files → INSERT symbols → INSERT refs → INSERT relationships
```

### 符号提取映射

| tree-sitter node type | SymbolKind |
|----------------------|------------|
| `function_item` | function |
| `struct_item` | struct |
| `enum_item` | enum |
| `trait_item` | trait |
| `impl_item` | impl |
| `type_item` | type_alias |
| `const_item` | const |
| `static_item` | static |
| `mod_item` | mod |
| `macro_definition` | macro |

### 调用关系提取

树遍历 `call_expression` 节点：
- `fn_name(args)` → 直接函数调用，按名称匹配同文件/同 crate 符号
- `x.method(args)` → 查「类型名 → impl 块方法」映射表（同 crate 范围内构建）
- `module::fn(args)` → 按完整路径匹配符号
- `Self::method(args)` → 当前 impl 块内方法调用

### 增量索引

```
old_files (from index)  ─┐
                          ├─→ added    → parse + insert
new_files (from disk)    ─┘  removed  → delete symbols
                              modified → delete old + re-parse + insert
                              same     → skip
```

### 懒生命周期

```
首次查询 → 引擎不存在 → 创建 CodegraphEngine
  → .codegraph/index.db 不存在 → 全量索引（进度输出）
  → .codegraph/index.db 已存在 → 增量更新（哈希比对）
  → 启动 FileWatcher
引擎已存在 → 直接查询
FileWatcher 10 分钟无查询 → 自动 stop（下次查询重新唤醒）
```

## 4. 查询引擎

- `codegraph_node(symbol)` → 符号定义位置、签名、可见性、references、callers/callees（默认 depth=1）
- `codegraph_explore(query)` → 关键词匹配符号 + 返回相关调用路径
- `get_callers(symbol, depth)` / `get_callees(symbol, depth)` → 传递闭包，默认 depth=2，最大 5
- `fuzzy_find(name)` → Levenshtein 距离 ≤ 3 的相似符号建议

## 5. 错误处理

| Level | 场景 | 处理 |
|-------|------|------|
| 1 | Engine 未初始化 | 创建引擎 → 按需索引 |
| 2 | 索引不存在 | 自动全量索引 + 进度 |
| 3 | 部分文件解析失败 | tree-sitter 容错 + warning 计数 |
| 4 | 查询无结果 | not_found + 模糊建议 + lsp.rs fallback |
| 5 | SQLite 损坏/锁定 | WAL 防锁；损坏时提示 clean → reindex |

**Confidence 标注**：
- `high`：同文件直接调用
- `medium`：同 crate 跨文件通过 impl 映射表解析
- `low`：仅名称匹配，无类型信息
- `unresolved`：trait 方法无法解析

## 6. 测试策略

| 层 | 内容 |
|----|------|
| 单元 | types JSON 往返；IndexStore CRUD（内存 SQLite）；tree-sitter 符号/关系提取；调用图解析 |
| 集成 | 以 wgenty-code 自身为 fixture：全量索引→查询、增量索引、depth 查询、MCP 协议 |
| CI | `cargo clippy --all-targets -- -D warnings` + `cargo fmt -- --check` |

## 7. 与 lsp.rs 整合

```
codegraph_node/explore 执行:
  1. 检查引擎是否存在 & 索引是否可用
  2. 可用 → 使用 codegraph 索引查询（带 confidence 标注）
  3. 不可用 → 回退到 lsp.rs regex 查找
  4. 结果标注来源："[codegraph]" vs "[regex fallback]"
```
