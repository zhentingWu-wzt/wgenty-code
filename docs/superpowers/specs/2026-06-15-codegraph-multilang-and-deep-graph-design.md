---
comet_change: codegraph-multilang-and-deep-graph
role: technical-design
canonical_spec: openspec
---

# Codegraph Multilang & Deep Graph — 技术设计

## 1. OQ 确认

| OQ | 决策 |
|-----|------|
| 1 Java lambda | A: 忽略/降级 Contains |
| 2 Python 动态类型 | A: 仅静态标注 |
| 3 性能退化 | A: 接受 ≤1.5× baseline |
| 4 Async need | A: sync trait + spawn_blocking |

## 2. 架构

```
src/tools/codegraph/
├── adapters/
│   ├── mod.rs        LanguageAdapter trait 定义
│   ├── rust.rs        RustAdapter (重构自现有 parser.rs)
│   ├── java.rs        JavaAdapter (tree-sitter-java)
│   └── python.rs      PythonAdapter (tree-sitter-python)
├── parser.rs          ParserPool (HashMap<lang, Arc<Mutex<Parser>>>)
├── migration.rs       [NEW] Schema version + auto-migration
├── types.rs           [MOD] RelKind 扩展 + language 字段
├── indexer.rs         [MOD] adapter map + 语言路由
├── store.rs           [MOD] 新 RelKind 存储 + migration hook
└── query.rs           [MOD] 新关系查询
```

### LanguageAdapter trait

```rust
pub trait LanguageAdapter: Send + Sync {
    fn language(&self) -> &'static str;
    fn parse(&self, source: &str) -> Result<Tree, ParseError>;
    fn extract_symbols(&self, tree: &Tree, source: &str, path: &str) -> Vec<Symbol>;
    fn extract_relationships(&self, tree: &Tree, source: &str, syms: &[Symbol]) -> Vec<Relationship>;
    fn file_extensions(&self) -> &[&str];
}
```

### Schema Migration (v1 → v2)

- ALTER TABLE symbols ADD COLUMN language TEXT DEFAULT 'rust'
- CREATE TABLE relationships_v2 (new rel_kind column wider)
- 迁移后在 .codegraph/ 写入 `.schema_version` 文件

## 3. 实现要点

- parser pool: HashMap<&str, Arc<Mutex<CodeParser>>> 按语言缓存
- adapter 注册: indexer 启动时注册 RustAdapter/JavaAdapter/PythonAdapter
- 文件路由: `.rs`→rust, `.java`→java, `.py`→python（大小写不敏感）
- Java lambda: 跳过匿名类/lambda 节点，仅提取 method_declaration/class_declaration
- Python 类型: 仅提取 `typed_parameter`/`return_type` 有值的节点
- confidence: Java/Python 新适配器产出的符号 confidence=medium（新适配器初期标记），后续成熟后升级 high

## 4. 风险

| 风险 | 缓解 |
|------|------|
| Schema 迁移数据丢失 | 迁移前备份 old index.db → .codegraph/index.db.v1.bak |
| 编译时间增长 | tree-sitter 依赖仅在 feature `multilang` 下编译（可选） |
| Java/Python tree-sitter 质量 | 样例项目验证 ≥70% coverage |

## 5. Spec Patch

无。open 阶段 specs 已完整。

## 6. Migration

首次启动检测 schema version → 自动 ALTER TABLE → 写入 `.schema_version`。回退需手动删除 `.codegraph/` 并重建索引。
