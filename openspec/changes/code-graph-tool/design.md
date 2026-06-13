## Context

`wgenty-code` 当前通过 `tools/meta/lsp.rs` 提供基于正则表达式的符号查找能力。该方案约覆盖 80% 的 LSP 功能但存在根本性局限：无法构建真正的 AST、无调用图分析、正则误匹配率高。AI agent 在执行复杂代码任务时需要精确的代码理解基础设施。

本设计基于 tree-sitter 实现离线静态索引 + 可选 LSP 实时补充的混合架构，为 agent 提供符号索引、引用追踪和调用图分析能力。

## Goals / Non-Goals

**Goals:**
- tree-sitter 驱动的 Rust 源码 AST 解析与符号提取
- SQLite 持久化索引：符号定义、引用、调用关系
- 符号查询 API：定义查找、引用追踪（`codegraph_node`）
- 符号探索 API：按查询返回相关符号及调用路径（`codegraph_explore`）
- 调用图分析：caller/callee，支持可配置深度的传递闭包
- 增量索引：文件变更检测，仅重索引变更文件
- 内置 Tool + MCP Server 双接口
- CLI 子命令：`codegraph index|query|clean`

**Non-Goals:**
- Rust 以外的语言支持（架构预留扩展点但不实现）
- 数据流分析、控制流图
- 宏展开追踪（tree-sitter 处理的是展开前的语法树）
- 跨仓库/workspace 级分析
- 图形化可视化
- 类型推断、泛型实例化追踪
- trait 方法实现的跨文件自动发现

## Decisions

### Decision 1: tree-sitter 作为解析引擎（而非 rust-analyzer / syn）

| 维度 | tree-sitter | rust-analyzer (LSP) | syn (Rust proc macro) |
|------|-------------|---------------------|------------------------|
| 解析速度 | 极快（增量解析 ms 级） | 慢（需要编译级分析） | 中等 |
| 容错性 | 优秀（错误恢复） | 依赖完整项目 | 差（要求合法语法） |
| 部署复杂度 | 低（单库 + grammar） | 高（需要 rust-analyzer 二进制） | 低 |
| 多语言扩展 | 原生支持 | 每语言一个 server | 仅 Rust |
| 语义精确度 | 语法级（非语义级） | 编译级（类型解析） | 语法级 |

**选择 tree-sitter**，理由：
- 语法级精确度满足符号索引 + 调用图需求
- 容错性强，无需完整编译环境
- 可作为通用索引引擎，后续扩展其他语言
- 与 `.codegraph/` 离线索引模式天然匹配

### Decision 2: SQLite 作为索引存储（而非 LMDB / 自定义二进制格式）

**选择 SQLite (rusqlite)**：
- WAL 模式支持读多写少并发，AI agent 查询场景匹配
- schema 灵活，后续增加关系类型无需迁移格式
- 与项目已有的 `rusqlite` 依赖一致（若不存在则新增）
- SQL 查询能力让复杂图遍历（传递闭包）直接用递归 CTE
- 单文件存储，易于分发和清理

### Decision 3: 双形态架构（内置 Tool + MCP Server）

```
┌─────────────────────────────────────────────────────┐
│                 codegraph 核心引擎                     │
│                                                       │
│  ┌──────────┐  ┌────────────┐  ┌──────────────────┐  │
│  │ Indexer  │  │QueryEngine │  │   IndexStore      │  │
│  │          │  │            │  │   (SQLite)        │  │
│  │ • parse  │  │ • symbol   │  │                   │  │
│  │ • extract│  │ • refs     │  │ symbols | refs    │  │
│  │ • store  │  │ • callgraph│  │ relationships     │  │
│  └────┬─────┘  └─────┬──────┘  └────────┬──────────┘  │
│       │              │                  │              │
│       └──────────────┼──────────────────┘              │
│                      │                                 │
├──────────────────────┼─────────────────────────────────┤
│              接口层   │                                 │
│       ┌──────────────┼──────────────┐                  │
│       ▼              ▼              ▼                  │
│  ┌─────────┐  ┌──────────┐  ┌──────────┐              │
│  │ CLI     │  │  Tool    │  │   MCP    │              │
│  │ codegraph│ │  impl    │  │  Server  │              │
│  │ index/   │  │ codegraph│  │ codegraph│              │
│  │ query/   │  │ _explore │  │ _explore │              │
│  │ clean    │  │ _node    │  │ _node    │              │
│  └─────────┘  └──────────┘  └──────────┘              │
└─────────────────────────────────────────────────────┘
```

核心引擎独立于接口层。内置 Tool 实现 `Tool` trait 直接调用引擎；MCP Server 通过 MCP 协议适配层暴露相同能力。

### Decision 4: 增量索引策略

使用文件内容哈希进行变更检测：
- 索引时为每个文件计算 SHA256，存入 `files` 表
- 下次索引时比对哈希，仅重索引变更文件
- 删除的符号通过比较新旧文件列表清理

增量流程：
```
old_files (from index)  ─┐
                          ├─→ added    → parse + insert
new_files (from disk)    ─┘  removed  → delete symbols
                              modified → delete old + re-parse + insert
                              same     → skip
```

### Decision 5: 调用图解析方法

tree-sitter 提供 AST 而非语义分析，调用解析采用最佳努力策略：

1. **直接函数调用**：`foo(args)` → 匹配 AST 中 `call_expression` 节点，按名称解析 callee
2. **方法调用**：`x.method(args)` → 结合 `impl` 块信息，尝试将 `method` 解析到对应类型的 impl
3. **跨模块调用**：`module::foo(args)` → 按路径解析，匹配 `use` 声明
4. **trait 方法调用**：`x.trait_method()` → v1 不做 trait 实现的跨文件解析，标注为 `unresolved_trait_method`

### Decision 6: 模块结构

```
src/tools/codegraph/
├── mod.rs                # 模块入口，re-export
├── indexer.rs            # 索引引擎（tree-sitter 驱动）
├── query.rs              # 查询引擎（SQLite 查询 + 图遍历）
├── store.rs              # IndexStore — SQLite schema 定义与 CRUD
├── types.rs              # 共享类型：Symbol, Reference, Relationship, SymbolKind
├── tool_explore.rs       # codegraph_explore Tool 实现
├── tool_node.rs          # codegraph_node Tool 实现
├── mcp.rs                # MCP 适配层（若独立于 tools/codegraph/）
└── cli.rs                # CLI codegraph index|query|clean 子命令
```

## Risks / Trade-offs

| Risk | Mitigation |
|------|------------|
| tree-sitter-rust grammar 与 rustc 语法版本不一致 | 锁定 tree-sitter 版本，CI 中验证 wgenty-code 自身可被成功索引 |
| 大项目全量索引耗时长（如 rust-lang/rust） | 增量索引为默认模式；全量索引显示进度条；后台线程执行不阻塞 UI |
| SQLite 不支持复杂图遍历 | 使用递归 CTE 实现传递闭包（depth-limited）；调用图查询限制最大深度 5 |
| 方法调用解析准确率 < 100%（缺乏类型推断） | 标注 `confidence` 字段（high/medium/low）；unresolved 调用标注为 `unresolved`；LSP fallback 补充 |
| 宏生成代码无法索引 | v1 明确跳过；后续可考虑 `cargo expand` 预处理 |
| MCP Server 与内置 Tool 行为不一致 | 共享同一 QueryEngine 实例；CI 添加对比测试 |
| 二进制体积增加 2-3 MB | 控制在 5% 增量内；tree-sitter 按需编译 language bindings |

## Open Questions

1. **LSP fallback 时机**：何时启用 LSP 实时查询补充？建议：仅在索引未覆盖或 `confidence=low` 时触发，用户可通过 CLI flag 控制
2. **跨文件 impl block 解析**：trait impl 可能在不同文件中。v1 是否全量扫描关联文件？建议：v1 仅关联同 crate 文件，标注 `cross_crate: unresolved`
3. **索引自动触发**：agent 使用 codegraph 工具时，索引缺失是否自动触发构建？建议：返回错误提示用户手动运行 `codegraph index`，不做隐式触发
