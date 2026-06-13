## 1. 项目基础设施

- [x] 1.1 添加 tree-sitter、tree-sitter-rust、rusqlite 依赖到 Cargo.toml
- [x] 1.2 创建 `src/tools/codegraph/` 模块目录结构和 `mod.rs` 入口
- [x] 1.3 在 `src/tools/mod.rs` 中注册 codegraph 模块

## 2. 数据模型与存储层

- [x] 2.1 定义核心类型：`Symbol`, `SymbolKind`, `Reference`, `Relationship`, `Visibility` 在 `types.rs`
- [x] 2.2 实现 `IndexStore`：SQLite schema 创建（symbols, references, relationships, files 表）在 `store.rs`
- [x] 2.3 实现 `IndexStore::upsert_symbol` / `delete_symbol` / `get_symbol` CRUD 方法
- [x] 2.4 实现 `IndexStore::insert_reference` / `get_references` 引用管理
- [x] 2.5 实现 `IndexStore::insert_relationship` / `get_callers` / `get_callees` 关系查询
- [x] 2.6 实现 `IndexStore` 的文件哈希变更检测方法

## 3. 索引引擎

- [x] 3.1 实现 tree-sitter Rust parser 初始化和语言注入
- [x] 3.2 实现 AST 遍历器：提取函数、结构体、枚举、trait、impl、类型别名、常量、静态变量、模块定义
- [x] 3.3 实现符号引用提取：函数调用、类型引用、use 声明中的符号引用
- [x] 3.4 实现调用关系提取：call_expression → callee 映射，含方法调用解析
- [x] 3.5 实现全量索引流程：扫描所有 `.rs` 文件 → 解析 → 存储，含进度输出
- [x] 3.6 实现增量索引流程：文件哈希比对 → 仅重索引变更文件

## 4. 查询引擎

- [x] 4.1 实现 `codegraph_node` 查询：按名称查找符号定义、引用、callers/callees
- [x] 4.2 实现 `codegraph_explore` 查询：关键词匹配符号 + 返回相关调用路径
- [x] 4.3 实现传递闭包查询：`get_callers`/`get_callees` 带 depth 限制（默认2，最大5）
- [x] 4.4 实现模糊匹配：未找到符号时，按 Levenshtein 距离 ≤ 3 提供相似名称建议

## 5. 内置 Tool 实现

- [x] 5.1 实现 `CodegraphNodeTool`：Tool trait，调用 query 引擎的 `codegraph_node`
- [x] 5.2 实现 `CodegraphExploreTool`：Tool trait，调用 query 引擎的 `codegraph_explore`
- [x] 5.3 在 `ToolRegistry` 中注册两个 codegraph 工具为 read-only

## 6. CLI 命令

- [x] 6.1 在 `src/cli/args.rs` 中添加 `Codegraph` 子命令（index/query/clean）
- [x] 6.2 实现 `codegraph index` 命令：调用索引引擎的全量/增量索引
- [x] 6.3 实现 `codegraph query <symbol>` 命令：CLI 下行 `codegraph_node` 查询
- [x] 6.4 实现 `codegraph clean` 命令：删除 `.codegraph/` 目录

## 7. MCP Server 集成

- [x] 7.1 实现 MCP 工具适配层：将 `codegraph_explore`/`codegraph_node` 包装为 MCP tools
- [x] 7.2 在 MCP 服务注册表中注册 codegraph MCP tools
- [x] 7.3 实现 MCP 查询时的索引存在性检查和错误消息

## 8. 与现有 lsp.rs 整合

- [x] 8.1 在 `lsp.rs` 中添加 codegraph 索引作为首选查询源（index-first, regex-fallback）
- [x] 8.2 确保 `lsp.rs` goToDefinition/findReferences 在 codegraph 可用时优先使用索引结果

## 9. 测试

- [x] 9.1 单元测试：types 序列化/反序列化
- [x] 9.2 单元测试：IndexStore CRUD 操作
- [x] 9.3 单元测试：tree-sitter 解析器符号提取（用 wgenty-code 自身源码作为测试输入）
- [x] 9.4 集成测试：全量索引 + codegraph_node 查询端到端
- [x] 9.5 集成测试：增量索引（修改文件 → 重新索引 → 验证仅有变更文件被更新）
- [x] 9.6 集成测试：调用图查询（callers/callees/depth）
- [x] 9.7 集成测试：MCP tools/list 和 tools/call
- [x] 9.8 验证：`cargo clippy --all-targets -- -D warnings` 和 `cargo fmt -- --check` 通过

## 10. 收尾

- [x] 10.1 将 `.codegraph/` 添加到项目 `.gitignore`
- [x] 10.2 更新 CLAUDE.md 中关于 CodeGraph 的说明，标注 codegraph 工具已可用
