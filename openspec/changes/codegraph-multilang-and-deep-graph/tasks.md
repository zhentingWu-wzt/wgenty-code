# Tasks — codegraph-multilang-and-deep-graph

## 1. 依赖 + LanguageAdapter trait

- [x] 1.1 Cargo.toml: 新增 tree-sitter-java, tree-sitter-python
- [ ] 1.2 创建 src/tools/codegraph/adapters/mod.rs: LanguageAdapter trait 定义
- [ ] 1.3 实现 RustAdapter (src/tools/codegraph/adapters/rust.rs)
- [ ] 1.4 实现 JavaAdapter (src/tools/codegraph/adapters/java.rs)
- [ ] 1.5 实现 PythonAdapter (src/tools/codegraph/adapters/python.rs)
- [ ] 1.6 单元测试：每种 adapter 解析简单代码片段

## 2. Types + RelKind 扩展

- [x] 2.1 types.rs: Symbol 增加 language 字段
- [x] 2.2 types.rs: RelKind 新增 Inherits/TypeOf/Returns/Parameter
- [x] 2.3 单元测试：新 RelKind 的 as_str/parse

## 3. Parser Pool + 路由

- [ ] 3.1 parser.rs: 重构为多语言 parser pool (HashMap<&str, Arc<Mutex<CodeParser>>>)
- [ ] 3.2 parser.rs: 文件扩展名→language 路由
- [ ] 3.3 单元测试：pool 缓存、语言路由正确性

## 4. Schema 迁移

- [ ] 4.1 创建 src/tools/codegraph/migration.rs: version 检测 + 自动迁移
- [ ] 4.2 store.rs: IndexStore::open() 时调用 migration
- [ ] 4.3 store.rs: 新增新 RelKind 的 insert/query 方法
- [ ] 4.4 单元测试：迁移前后数据完整性

## 5. Indexer 适配

- [ ] 5.1 indexer.rs: 注入 adapter map
- [ ] 5.2 indexer.rs: 按文件扩展名选择 adapter → extract_symbols/extract_relationships
- [ ] 5.3 indexer.rs: language 字段写入
- [ ] 5.4 单元测试：三语言各索引一个 fixture 文件

## 6. Query 适配

- [ ] 6.1 query.rs: 新 RelKind 查询（按 Inherits/TypeOf 过滤）
- [ ] 6.2 tools.rs: 现有 tool description 更新（mention Java/Python support）

## 7. 验收

- [ ] 7.1 Java 样例项目（≥100 文件）索引验证 coverage ≥70%
- [ ] 7.2 Python 样例项目（≥100 文件）索引验证 coverage ≥70%
- [ ] 7.3 bench-perf.sh 重跑对比 #0 基线（全量索引耗时 ≤ baseline × 1.5）
- [ ] 7.4 cargo build + cargo test 全绿
- [ ] 7.5 现有 Rust 索引数据向后兼容（不丢数据）

## 8. Spec 同步 + 归档

- [ ] 8.1 创建 delta specs: multilang-indexing/spec.md (ADDED)
- [ ] 8.2 创建 delta specs: symbol-graph-deep/spec.md (ADDED)
- [ ] 8.3 修改 delta specs: code-indexing/spec.md (MODIFIED)
- [ ] 8.4 openspec validate → comet-verify → comet-archive
