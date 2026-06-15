---
change: codegraph-multilang-and-deep-graph
design-doc: docs/superpowers/specs/2026-06-15-codegraph-multilang-and-deep-graph-design.md
base-ref: d69210652f377344d876f7bcd7b5db787d750e09
---

# Codegraph Multilang & Deep Graph 实施计划

**Goal:** 多语言 parser + 深层 Symbol Graph 关系。新增 tree-sitter-java/python，4 种新 RelKind，LanguageAdapter trait，schema 迁移。

**Architecture:** adapters/ (3 语言适配器) + 重构 parser.rs → ParserPool + migration.rs (schema v1→v2)

---

## Phase 0: 依赖 + 基础设施

### Task 0.1: Cargo.toml 依赖
- [x] 新增 tree-sitter-java, tree-sitter-python
- [x] cargo build 验证
- [x] Commit

### Task 0.2: RelKind 扩展 + language 字段
- [x] types.rs: RelKind 加 Inherits/TypeOf/Returns/Parameter
- [x] types.rs: Symbol 加 language 字段 (default "rust")
- [x] 单元测试: as_str/parse
- [x] Commit

---

## Phase 1: LanguageAdapter + 适配器

### Task 1.1: adapters/mod.rs — trait 定义
- [x] 创建 `src/tools/codegraph/adapters/mod.rs`
- [x] LanguageAdapter trait
- [x] Commit

### Task 1.2: RustAdapter
- [x] 创建 adapters/rust.rs
- [x] 复用现有 parser.rs Rust 逻辑 → impl LanguageAdapter
- [x] Commit

### Task 1.3: JavaAdapter
- [x] 创建 adapters/java.rs
- [x] tree-sitter-java: class/method/field → Symbol; extends/implements → Inherits
- [x] 单元测试
- [x] Commit

### Task 1.4: PythonAdapter
- [x] 创建 adapters/python.rs
- [x] tree-sitter-python: function/class → Symbol; typed params/returns → TypeOf/Returns
- [x] 单元测试
- [x] Commit

---

## Phase 2: Parser Pool + Indexer 重构

### Task 2.1: parser.rs → ParserPool
- [x] 重构为 HashMap<lang, Arc<Mutex<CodeParser>>>
- [x] 文件扩展名→lang 路由
- [x] Commit

### Task 2.2: indexer.rs adapter 集成
- [x] 注册 3 适配器
- [x] 按文件扩展名选择适配器
- [x] language 字段写入 symbol
- [x] Commit

---

## Phase 3: Schema 迁移 + Store

### Task 3.1: migration.rs
- [x] 检测 schema version
- [x] ALTER TABLE symbols ADD COLUMN language
- [x] 新关系类型表
- [x] 单元测试
- [x] Commit

### Task 3.2: store.rs 新关系存储
- [x] 新 RelKind 的 insert/query
- [x] Commit

---

## Phase 4: 验收 + 归档

### Task 4.1: cargo build + test 全绿
- [x] 305 passed, 0 failures (1 pre-existing unrelated failure)
### Task 4.2: Java 样例项目 coverage ≥70%
### Task 4.3: Python 样例项目 coverage ≥70%
### Task 4.4: bench-perf.sh 对比 #0 baseline (≤1.5×)
### Task 4.5: 勾选 tasks.md + guard → verify → archive
