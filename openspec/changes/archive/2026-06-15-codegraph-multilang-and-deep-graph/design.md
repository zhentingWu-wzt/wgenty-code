## Context

codegraph 当前仅支持 Rust 单语言解析（`tree-sitter-rust 0.24`），关系类型限于 4 种。parser.rs 和 indexer.rs 紧密耦合 Rust-specific 逻辑。Types 模型已通用化（Symbol/Relationship/Confidence 不绑语言），为新语言扩展奠定基础。

## Goals / Non-Goals

**Goals:**
- 新增 tree-sitter-java 和 tree-sitter-python，实现三语言深度解析
- 抽离 LanguageAdapter trait + 3 种语言适配器
- 新增 ≥4 种 RelKind（Inherits/TypeOf/Returns/Parameter）
- 索引 schema 演进（language 字段 + 新关系表）
- Java/Python 样例项目覆盖率 ≥70%

**Non-Goals:**
- 不新增第 4+ 种语言
- 不实现跨语言语义检索（如 "Java class A → Python class B"）
- 不修改 TUI / MCP / prompts

## Decisions

### D1: LanguageAdapter trait 设计

```rust
pub trait LanguageAdapter: Send + Sync {
    fn language(&self) -> &'static str; // "rust" | "java" | "python"
    fn parse(&self, source: &str) -> Result<Tree, ParseError>;
    fn extract_symbols(&self, tree: &Tree, source: &str, file_path: &str) -> Vec<Symbol>;
    fn extract_relationships(&self, tree: &Tree, source: &str, symbols: &[Symbol]) -> Vec<Relationship>;
    fn file_extensions(&self) -> &[&str]; // ["rs"] | ["java"] | ["py"]
}
```

### D2: 文件扩展名路由

`indexer.rs` 根据文件扩展名选择 adapter：`.rs`→RustAdapter, `.java`→JavaAdapter, `.py`→PythonAdapter。其余文件跳过。

### D3: 新 RelKind

在 `RelKind` 枚举中增加：`Inherits`（extends/implements）、`TypeOf`（变量:类型）、`Returns`（函数→返回类型）、`Parameter`（函数→参数类型）。

### D4: Schema 迁移策略

`migration.rs` 版本化管理：version=1 为当前 schema；version=2 增加 language 列 + 新关系表。IndexStore::open() 时检测并自动迁移。

### D5: parser pool

Parser 按语言缓存（HashMap<&str, Arc<Mutex<CodeParser>>>），避免重复初始化 tree-sitter grammar。

## Risks

| 风险 | 缓解 |
|------|------|
| Schema 迁移不可逆 | migration.rs 版本化 + --dry-run |
| 多语言编译时间长 | feature flags 可选 language |
| Java/Python tree-sitter 质量 | 样例项目验证 ≥70% coverage |
| 新 RelKind 查询复杂度 | 复用现有 call_path/explore 逻辑 |

## Migration Plan

首次启动自动检测 schema version 并迁移。降级不支持（需重新 `codegraph index --full`）。

## Open Questions (build 阶段 brainstorming)
1. Java lambda/匿名类如何表示？(tree-sitter-java 的支持程度)
2. Python 动态类型如何映射到 TypeOf？（仅标注静态可推导的类型）
3. 索引性能退化多少？（bench-perf.sh 重跑对比）
4. LanguageAdapter 是否需要 async？（tree-sitter parse 是 CPU-bound，sync 即可）
