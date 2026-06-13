# Brainstorm Summary

- Change: code-graph-tool
- Date: 2026-06-13

## 确认的技术方案

| # | 决策点 | 选择 |
|---|--------|------|
| 1 | 索引触发策略 | 惰性索引 + 空闲自动休眠：首次查询触发全量索引，之后 FileWatcher 增量更新，10 分钟无查询后自动停止监听释放资源 |
| 2 | 引擎共享架构 | AppState 中 `Arc<CodegraphEngine>` 单例，Tool/MCP/CLI 三层共享同一实例（参考 Claude Code CodeGraph 单例模式） |
| 3 | tree-sitter 集成 | 方案 A：`tree-sitter` + `tree-sitter-rust` static crate，一步编译；`parser_pool = num_cpus - 1` 用于并行全量索引 |
| 4 | 调用图提取 | 方案 B：同 crate 扫描建立「类型名 → impl 块所含方法」映射表，跨文件解析方法调用，confidence 分 high/medium/low/unresolved 四级 |
| 5 | 查询输出格式 | 方案 A：Markdown 文本格式（agent 友好），ToolOutput.metadata 附带结构化统计字段 |
| 6 | 数据模型 | SQLite (WAL 模式)：files / symbols / refs / relationships 四表，递归 CTE 实现传递闭包查询 |
| 7 | 索引流程 | WalkDir 扫描 → SHA256 哈希比对 → parallel parse (parser_pool) → 单事务 batch insert |
| 8 | 错误处理 | 5 级分层：Engine 未初始化 → 索引不存在 → 部分解析失败 → 查询无结果 → SQLite 损坏 |
| 9 | 测试策略 | 单元测试（内存 SQLite + tree-sitter 内联代码）+ 集成测试（wgenty-code 自身为 fixture）+ CI |

## 关键取舍与风险

- **tree-sitter vs rust-analyzer**：选 tree-sitter 牺牲语义精度换取容错性和部署简洁性
- **调用图准确率**：v1 不依赖类型推断，~85% 准确率（同 crate 范围内）。trait 方法标注为 unresolved 不回退猜测
- **宏展开跳过**：v1 不索引宏生成代码，tree-sitter 处理的是展开前的 AST
- **二进制体积 +2-3 MB**：tree-sitter 运行时 + Rust grammar 静态链接

## 测试策略

- 单元：types 序列化、IndexStore CRUD、tree-sitter 符号/关系提取、调用图解析
- 集成：以 wgenty-code 自身源码为 fixture，端到端验证全量索引、增量索引、调用图深度查询、MCP 协议合规
- CI：clippy + fmt 零警告

## Spec Patch

1. **code-indexing/spec.md** — 新增 Requirement: Parallel indexing（parser_pool 并行解析）
2. **symbol-query/spec.md** — 新增 Requirement: Index-first query strategy（codegraph 优先于 regex fallback）
