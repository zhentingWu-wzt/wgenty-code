## Why

当前 `tools/meta/lsp.rs` 基于正则表达式做符号查找，无法提供精确的 AST 级符号索引和调用图分析能力。AI agent 在处理复杂代码任务时需要精确理解代码结构（"谁调用了这个函数？""这个 trait 有哪些实现？"），正则方案误匹配率高且完全缺失关系图谱。引入 tree-sitter 静态索引 + LSP 实时查询的混合方案，为 agent 提供生产级的代码理解基础设施。

## What Changes

- **新增** tree-sitter 驱动的代码索引引擎，支持全量和增量索引
- **新增** 基于 SQLite 的符号存储：定义、引用、调用关系
- **新增** `codegraph_explore` 工具：按查询返回相关符号及其调用路径
- **新增** `codegraph_node` 工具：返回单个符号的定义、签名、callers/callees
- **新增** MCP Server 形态的 codegraph 接口，供外部客户端使用
- **新增** CLI 子命令 `codegraph index|query|clean`
- **升级** 现有 `lsp.rs` 的符号查找能力，与 codegraph 索引互通
- `.codegraph/` 目录新增为项目级索引存储

## Capabilities

### New Capabilities

- `code-indexing`: tree-sitter 驱动的 Rust 源码 AST 解析与符号索引构建，支持全量和增量更新
- `symbol-query`: 精确的符号定义查找与引用追踪，按名称/类型/位置检索
- `call-graph`: 函数级调用图分析，支持 caller（谁调用我）和 callee（我调用谁）双向查询
- `codegraph-mcp`: MCP 协议接口，将 codegraph 查询能力暴露为 MCP tools，供外部 AI 客户端使用

### Modified Capabilities

<!-- 当前 openspec/specs/ 下无已有 spec，无需修改现有能力 -->

## Impact

- **依赖新增**: `tree-sitter`、`tree-sitter-rust`、`rusqlite`（或等效 SQLite binding）
- **新增模块**: `src/tools/codegraph/`（索引引擎、查询引擎、MCP 适配层）
- **影响模块**: `src/tools/meta/lsp.rs`（与 codegraph 索引整合）、`src/tools/mod.rs`（注册新工具）
- **CLI 新增**: `src/cli/args.rs` 新增 `Codegraph` 子命令
- **存储新增**: 项目根目录 `.codegraph/` 目录（可加入 `.gitignore`）
- **二进制体积**: 预计增加 ~2-3 MB（tree-sitter 运行时 + Rust grammar）
