```yaml
---
change: code-graph-tool
design-doc: docs/superpowers/specs/2026-06-13-code-graph-tool-design.md
base-ref: 573e04b075bb7c9e8b80c533aff3646733bbb913
---
```

# CodeGraph 代码图谱工具 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a tree-sitter-based persistent code index (SQLite) with symbol lookup, reference tracking, transitive call graph queries, and fuzzy search -- exposed via built-in tools, CLI commands, and MCP protocol.

**Architecture:** Three-layer design: (1) **Indexer** uses tree-sitter to parse `.rs` files and extract symbols/refs/relationships into a SQLite store; (2) **QueryEngine** provides `codegraph_node`, `codegraph_explore`, `get_callers`/`get_callees` with transitive closure, and fuzzy fallback when no index exists; (3) **Interface layer** wraps the engine into built-in `Tool` trait impls, CLI subcommands (`codegraph index/query/clean`), and auto-exposes via MCP. The engine lives under `AppState` and is lazy-initialized on first query.

**Tech Stack:** tree-sitter 0.24 + tree-sitter-rust for parsing, rusqlite 0.31 (bundled SQLite) for persistent storage, walkdir for file scanning, sha2 for content hashing.

---

### Task 1: 项目基础设施 -- 依赖和模块脚手架

**Files:**
- Modify: `Cargo.toml`
- Create: `src/tools/codegraph/mod.rs`
- Modify: `src/tools/mod.rs`

- [x] **Step 1.1: 往 Cargo.toml 添加 tree-sitter 和 rusqlite 依赖**

在 `[dependencies]` 段（bytes 后面）插入：

```toml
# Code indexing & graph queries
tree-sitter = "0.24"
tree-sitter-rust = "0.24"
rusqlite = { version = "0.31", features = ["bundled"] }
```

验证：
```bash
cargo check 2>&1 | head -20
```
预期：新依赖解析成功。

- [x] **Step 1.2: 创建 codegraph 模块目录和入口 mod.rs**

文件：`src/tools/codegraph/mod.rs`

```rust
//! CodeGraph -- persistent code index and query engine.
//!
//! Architecture:
//!   types.rs     -- core data types (Symbol, SymbolKind, Reference, etc.)
//!   store.rs     -- IndexStore: SQLite-backed persistence layer
//!   parser.rs    -- tree-sitter parser initialization and AST traversal
//!   indexer.rs   -- full + incremental indexing with progress
//!   query.rs     -- QueryEngine: codegraph_node, codegraph_explore, call graph
//!   tools.rs     -- Tool trait wrappers (CodegraphNodeTool, CodegraphExploreTool)

pub mod types;
pub mod store;
pub mod parser;
pub mod indexer;
pub mod query;
pub mod tools;

use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Central engine holding indexer + query engine + store.
/// Shared across builtin tools and CLI via `Arc<RwLock<Option<CodegraphEngine>>>`.
pub struct CodegraphEngine {
    pub project_root: PathBuf,
    pub store: Arc<store::IndexStore>,
    pub indexer: Arc<indexer::Indexer>,
    pub query_engine: Arc<query::QueryEngine>,
}

impl CodegraphEngine {
    /// Create a new engine, optionally running an initial index.
    /// When `auto_index` is true and the index doesn't exist yet,
    /// a full index is performed synchronously (with progress to stderr).
    pub fn new(project_root: PathBuf, auto_index: bool) -> anyhow::Result<Self> {
        let store = Arc::new(store::IndexStore::open(&project_root)?);
        let parser = Arc::new(std::sync::Mutex::new(parser::CodeParser::new()));
        let indexer = Arc::new(indexer::Indexer::new(store.clone(), parser));
        let query_engine = Arc::new(query::QueryEngine::new(store.clone()));

        if auto_index && !store.has_index()? {
            eprintln!("[codegraph] No index found, running full indexing...");
            indexer.index_full(&project_root)?;
            eprintln!("[codegraph] Indexing complete.");
        }

        Ok(Self {
            project_root,
            store,
            indexer,
            query_engine,
        })
    }

    /// Ensure the index is up-to-date (incremental refresh).
    /// Called before every query if the engine exists.
    pub fn refresh(&self) -> anyhow::Result<()> {
        self.indexer.index_incremental(&self.project_root)
    }

    /// Check whether the index DB file exists on disk.
    pub fn has_index(&self) -> bool {
        self.store.has_index().unwrap_or(false)
    }
}
```

- [x] **Step 1.3: 在 `src/tools/mod.rs` 注册 codegraph 模块**

添加模块声明（在 `pub mod meta;` 后面）：
```rust
pub mod codegraph;
```

在文件底部的 re-export 中添加：
```rust
pub use codegraph::{
    tools::{CodegraphExploreTool, CodegraphNodeTool},
    CodegraphEngine,
};
```

验证：
```bash
cargo check 2>&1 | head -30
```
预期：模块解析成功（子模块尚为空，有 warning 属于正常）。

- [x] **Step 1.4: 提交**

```bash
git add Cargo.toml src/tools/codegraph/mod.rs src/tools/mod.rs
git commit -m "feat(codegraph): add dependencies, module scaffold"
```

---

### Task 2: 核心数据类型

**Files:**
- Create: `src/tools/codegraph/types.rs`

- [x] **Step 2.1: 定义 `SymbolKind` 枚举**

```rust
use serde::{Deserialize, Serialize};

/// Kinds of symbols the indexer can extract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SymbolKind {
    Function,
    Struct,
    Enum,
    Trait,
    Impl,
    TypeAlias,
    Const,
    Static,
    Mod,
    Macro,
}

impl SymbolKind {
    /// Parse a tree-sitter node type string into a SymbolKind.
    pub fn from_node_type(node_type: &str) -> Option<Self> {
        match node_type {
            "function_item" => Some(Self::Function),
            "struct_item" => Some(Self::Struct),
            "enum_item" => Some(Self::Enum),
            "trait_item" => Some(Self::Trait),
            "impl_item" => Some(Self::Impl),
            "type_item" => Some(Self::TypeAlias),
            "const_item" => Some(Self::Const),
            "static_item" => Some(Self::Static),
            "mod_item" => Some(Self::Mod),
            "macro_definition" => Some(Self::Macro),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Function => "function",
            Self::Struct => "struct",
            Self::Enum => "enum",
            Self::Trait => "trait",
            Self::Impl => "impl",
            Self::TypeAlias => "type_alias",
            Self::Const => "const",
            Self::Static => "static",
            Self::Mod => "mod",
            Self::Macro => "macro",
        }
    }
}
```

- [x] **Step 2.2: 定义 `Visibility` 枚举**

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Visibility {
    Pub,
    PubCrate,
    PubSuper,
    Private,
}

impl Visibility {
    pub fn from_visibility_modifier(modifier: &str) -> Self {
        match modifier {
            "pub" => Self::Pub,
            "pub(crate)" => Self::PubCrate,
            "pub(super)" => Self::PubSuper,
            _ => Self::Private,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pub => "pub",
            Self::PubCrate => "pub(crate)",
            Self::PubSuper => "pub(super)",
            Self::Private => "private",
        }
    }
}
```

- [x] **Step 2.3: 定义核心结构体：`Symbol`、`Reference`、`Relationship`**

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Symbol {
    pub id: Option<i64>,
    pub name: String,
    pub kind: SymbolKind,
    pub file_path: String,
    pub line: usize,
    pub col: usize,
    pub signature: Option<String>,
    pub visibility: Visibility,
    pub parent_module: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reference {
    pub id: Option<i64>,
    pub symbol_id: i64,
    pub file_path: String,
    pub line: usize,
    pub col: usize,
    pub ref_kind: RefKind,
    pub context: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RefKind {
    Call,
    TypeRef,
    Import,
    MethodCall,
}

impl RefKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Call => "call",
            Self::TypeRef => "type_ref",
            Self::Import => "import",
            Self::MethodCall => "method_call",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "call" => Self::Call,
            "type_ref" => Self::TypeRef,
            "import" => Self::Import,
            "method_call" => Self::MethodCall,
            _ => Self::Call,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Relationship {
    pub id: Option<i64>,
    pub source_id: i64,
    pub target_id: i64,
    pub rel_kind: RelKind,
    pub file_path: String,
    pub line: usize,
    pub confidence: Confidence,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RelKind {
    Calls,
    Implements,
    Contains,
    Imports,
}

impl RelKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Calls => "calls",
            Self::Implements => "implements",
            Self::Contains => "contains",
            Self::Imports => "imports",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "calls" => Self::Calls,
            "implements" => Self::Implements,
            "contains" => Self::Contains,
            "imports" => Self::Imports,
            _ => Self::Calls,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Confidence {
    High,
    Medium,
    Low,
    Unresolved,
}

impl Confidence {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::High => "high",
            Self::Medium => "medium",
            Self::Low => "low",
            Self::Unresolved => "unresolved",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "high" => Self::High,
            "medium" => Self::Medium,
            "low" => Self::Low,
            _ => Self::Unresolved,
        }
    }
}
```

- [x] **Step 2.4: 编写并运行单元测试（JSON 往返 + 枚举映射）**

追加到 `types.rs` 底部：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_symbol_kind_roundtrip() {
        let kinds = vec![
            SymbolKind::Function, SymbolKind::Struct, SymbolKind::Enum,
            SymbolKind::Trait, SymbolKind::Impl, SymbolKind::TypeAlias,
            SymbolKind::Const, SymbolKind::Static, SymbolKind::Mod, SymbolKind::Macro,
        ];
        for kind in &kinds {
            let json = serde_json::to_string(kind).unwrap();
            let deserialized: SymbolKind = serde_json::from_str(&json).unwrap();
            assert_eq!(*kind, deserialized);
        }
    }

    #[test]
    fn test_visibility_roundtrip() {
        let visibilities = vec![
            Visibility::Pub, Visibility::PubCrate,
            Visibility::PubSuper, Visibility::Private,
        ];
        for v in &visibilities {
            let json = serde_json::to_string(v).unwrap();
            let deserialized: Visibility = serde_json::from_str(&json).unwrap();
            assert_eq!(*v, deserialized);
        }
    }

    #[test]
    fn test_symbol_serialization() {
        let sym = Symbol {
            id: Some(1),
            name: "foo".into(),
            kind: SymbolKind::Function,
            file_path: "src/lib.rs".into(),
            line: 10, col: 1,
            signature: Some("fn foo(x: i32) -> bool".into()),
            visibility: Visibility::Pub,
            parent_module: Some("my_module".into()),
        };
        let json = serde_json::to_string_pretty(&sym).unwrap();
        let deserialized: Symbol = serde_json::from_str(&json).unwrap();
        assert_eq!(sym.name, deserialized.name);
    }

    #[test]
    fn test_kind_from_node_type() {
        assert_eq!(SymbolKind::from_node_type("function_item"), Some(SymbolKind::Function));
        assert_eq!(SymbolKind::from_node_type("struct_item"), Some(SymbolKind::Struct));
        assert_eq!(SymbolKind::from_node_type("unknown"), None);
    }
}
```

```bash
cargo test -- tools::codegraph::types::tests 2>&1
```
预期：4 个测试通过。

- [x] **Step 2.5: 提交**

```bash
git add src/tools/codegraph/types.rs
git commit -m "feat(codegraph): define core types (Symbol, Reference, Relationship, enums)"
```

---

### Task 3: IndexStore -- SQLite 持久层

**Files:**
- Create: `src/tools/codegraph/store.rs`

- [x] **Step 3.1: 实现 `IndexStore` 结构和 SQLite schema 创建**

文件 `src/tools/codegraph/store.rs`：

```rust
use crate::tools::codegraph::types::{
    Confidence, RefKind, RelKind, Reference, Relationship, Symbol,
};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use sha2::{Sha256, Digest};
use std::path::{Path, PathBuf};
use std::fs;

/// SQLite-backed persistent store for the codegraph index.
///
/// Schema:
///   files(id, path, sha256, indexed_at)
///   symbols(id, name, kind, file_id, line, col, signature, visibility, parent_module)
///   refs(id, symbol_id, file_id, line, col, ref_kind, context)
///   relationships(id, source_id, target_id, rel_kind, file_id, line, confidence)
pub struct IndexStore {
    conn: Connection,
    db_path: PathBuf,
}

impl IndexStore {
    /// Open (or create) the index database at `<project_root>/.codegraph/index.db`.
    /// Enables WAL mode for concurrent-read safety.
    pub fn open(project_root: &Path) -> anyhow::Result<Self> {
        let codegraph_dir = project_root.join(".codegraph");
        fs::create_dir_all(&codegraph_dir)?;
        let db_path = codegraph_dir.join("index.db");
        let conn = Connection::open(&db_path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        let mut store = Self { conn, db_path };
        store.create_schema()?;
        Ok(store)
    }

    fn create_schema(&self) -> anyhow::Result<()> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS files (
                id          INTEGER PRIMARY KEY,
                path        TEXT NOT NULL UNIQUE,
                sha256      TEXT NOT NULL,
                indexed_at  TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE TABLE IF NOT EXISTS symbols (
                id              INTEGER PRIMARY KEY,
                name            TEXT NOT NULL,
                kind            TEXT NOT NULL,
                file_id         INTEGER NOT NULL REFERENCES files(id),
                line            INTEGER NOT NULL,
                col             INTEGER NOT NULL,
                signature       TEXT,
                visibility      TEXT,
                parent_module   TEXT
            );
            CREATE TABLE IF NOT EXISTS refs (
                id          INTEGER PRIMARY KEY,
                symbol_id   INTEGER NOT NULL REFERENCES symbols(id),
                file_id     INTEGER NOT NULL REFERENCES files(id),
                line        INTEGER NOT NULL,
                col         INTEGER NOT NULL,
                ref_kind    TEXT NOT NULL,
                context     TEXT
            );
            CREATE TABLE IF NOT EXISTS relationships (
                id          INTEGER PRIMARY KEY,
                source_id   INTEGER NOT NULL REFERENCES symbols(id),
                target_id   INTEGER NOT NULL REFERENCES symbols(id),
                rel_kind    TEXT NOT NULL,
                file_id     INTEGER NOT NULL REFERENCES files(id),
                line        INTEGER NOT NULL,
                confidence  TEXT NOT NULL DEFAULT 'high'
            );
            CREATE INDEX IF NOT EXISTS idx_symbols_name ON symbols(name);
            CREATE INDEX IF NOT EXISTS idx_symbols_file ON symbols(file_id);
            CREATE INDEX IF NOT EXISTS idx_refs_symbol ON refs(symbol_id);
            CREATE INDEX IF NOT EXISTS idx_rels_source ON relationships(source_id);
            CREATE INDEX IF NOT EXISTS idx_rels_target ON relationships(target_id);"
        )?;
        Ok(())
    }

    pub fn has_index(&self) -> anyhow::Result<bool> {
        if !self.db_path.exists() { return Ok(false); }
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM files", [], |row| row.get(0),
        )?;
        Ok(count > 0)
    }
}
```

- [x] **Step 3.2: 实现文件哈希变更检测**

```rust
impl IndexStore {
    /// Compute SHA-256 of file contents.
    pub fn file_hash(path: &Path) -> anyhow::Result<String> {
        let contents = fs::read(path)?;
        let mut hasher = Sha256::new();
        hasher.update(&contents);
        Ok(format!("{:x}", hasher.finalize()))
    }

    pub fn get_file_hash(&self, path: &str) -> anyhow::Result<Option<String>> {
        let mut stmt = self.conn.prepare("SELECT sha256 FROM files WHERE path = ?1")?;
        let mut rows = stmt.query_map(params![path], |row| row.get(0))?;
        match rows.next() {
            Some(Ok(hash)) => Ok(Some(hash)),
            _ => Ok(None),
        }
    }

    pub fn get_all_files(&self) -> anyhow::Result<Vec<(String, String)>> {
        let mut stmt = self.conn.prepare("SELECT path, sha256 FROM files")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut result = Vec::new();
        for row in rows { result.push(row?); }
        Ok(result)
    }
}
```

- [x] **Step 3.3: 实现符号 CRUD 操作**

```rust
impl IndexStore {
    pub fn insert_symbol(&self, sym: &Symbol, file_id: i64) -> anyhow::Result<i64> {
        self.conn.execute(
            "INSERT INTO symbols (name, kind, file_id, line, col, signature, visibility, parent_module)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![sym.name, sym.kind.as_str(), file_id, sym.line as i64, sym.col as i64,
                    sym.signature, sym.visibility.as_str(), sym.parent_module],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn delete_symbols_for_file(&self, file_id: i64) -> anyhow::Result<()> {
        self.conn.execute("DELETE FROM refs WHERE file_id = ?1", params![file_id])?;
        self.conn.execute("DELETE FROM relationships WHERE file_id = ?1", params![file_id])?;
        self.conn.execute("DELETE FROM symbols WHERE file_id = ?1", params![file_id])?;
        Ok(())
    }

    pub fn get_symbol_by_name(&self, name: &str) -> anyhow::Result<Vec<Symbol>> {
        let mut stmt = self.conn.prepare(
            "SELECT s.id, s.name, s.kind, f.path, s.line, s.col, s.signature, s.visibility, s.parent_module
             FROM symbols s JOIN files f ON s.file_id = f.id
             WHERE s.name = ?1 ORDER BY f.path, s.line"
        )?;
        let rows = stmt.query_map(params![name], Self::map_symbol)?;
        let mut symbols = Vec::new();
        for row in rows { symbols.push(row?); }
        Ok(symbols)
    }

    pub fn get_symbol_by_id(&self, id: i64) -> anyhow::Result<Option<Symbol>> {
        let mut stmt = self.conn.prepare(
            "SELECT s.id, s.name, s.kind, f.path, s.line, s.col, s.signature, s.visibility, s.parent_module
             FROM symbols s JOIN files f ON s.file_id = f.id WHERE s.id = ?1"
        )?;
        let mut rows = stmt.query_map(params![id], Self::map_symbol)?;
        Ok(rows.next().transpose()?)
    }

    pub fn get_symbols_for_file(&self, file_id: i64) -> anyhow::Result<Vec<Symbol>> {
        let mut stmt = self.conn.prepare(
            "SELECT s.id, s.name, s.kind, f.path, s.line, s.col, s.signature, s.visibility, s.parent_module
             FROM symbols s JOIN files f ON s.file_id = f.id
             WHERE s.file_id = ?1 ORDER BY s.line"
        )?;
        let rows = stmt.query_map(params![file_id], Self::map_symbol)?;
        let mut symbols = Vec::new();
        for row in rows { symbols.push(row?); }
        Ok(symbols)
    }

    fn map_symbol(row: &rusqlite::Row) -> rusqlite::Result<Symbol> {
        Ok(Symbol {
            id: Some(row.get(0)?),
            name: row.get(1)?,
            kind: serde_json::from_str(&format!("\"{}\"", row.get::<_, String>(2)?)).unwrap(),
            file_path: row.get(3)?,
            line: row.get::<_, i64>(4)? as usize,
            col: row.get::<_, i64>(5)? as usize,
            signature: row.get(6)?,
            visibility: serde_json::from_str(&format!("\"{}\"", row.get::<_, String>(7)?)).unwrap(),
            parent_module: row.get(8)?,
        })
    }
}
```

- [x] **Step 3.4: 实现引用和关系 CRUD + 传递闭包查询**

```rust
impl IndexStore {
    pub fn insert_reference(&self, reference: &Reference, file_id: i64) -> anyhow::Result<i64> {
        self.conn.execute(
            "INSERT INTO refs (symbol_id, file_id, line, col, ref_kind, context)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![reference.symbol_id, file_id, reference.line as i64, reference.col as i64,
                    reference.ref_kind.as_str(), reference.context],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn get_references(&self, symbol_id: i64) -> anyhow::Result<Vec<Reference>> {
        let mut stmt = self.conn.prepare(
            "SELECT r.id, r.symbol_id, f.path, r.line, r.col, r.ref_kind, r.context
             FROM refs r JOIN files f ON r.file_id = f.id
             WHERE r.symbol_id = ?1 ORDER BY f.path, r.line"
        )?;
        let rows = stmt.query_map(params![symbol_id], |row| {
            Ok(Reference {
                id: Some(row.get(0)?),
                symbol_id: row.get(1)?,
                file_path: row.get(2)?,
                line: row.get::<_, i64>(3)? as usize,
                col: row.get::<_, i64>(4)? as usize,
                ref_kind: RefKind::from_str(&row.get::<_, String>(5)?),
                context: row.get(6)?,
            })
        })?;
        let mut refs = Vec::new();
        for row in rows { refs.push(row?); }
        Ok(refs)
    }

    pub fn insert_relationship(&self, rel: &Relationship) -> anyhow::Result<i64> {
        self.conn.execute(
            "INSERT INTO relationships (source_id, target_id, rel_kind, file_id, line, confidence)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![rel.source_id, rel.target_id, rel.rel_kind.as_str(),
                    rel.file_id, rel.line as i64, rel.confidence.as_str()],
        )?;
        Ok(self.conn.last_insert_rowid())
    }
}
```

传递闭包查询：

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SymbolCallEntry {
    pub id: i64,
    pub name: String,
    pub kind: crate::tools::codegraph::types::SymbolKind,
    pub file_path: String,
    pub line: usize,
    pub depth: u32,
}

impl IndexStore {
    pub fn get_callers(&self, target_id: i64, depth: u32) -> anyhow::Result<Vec<SymbolCallEntry>> {
        if depth <= 1 { return self.get_direct_callers(target_id); }
        let mut stmt = self.conn.prepare(
            "WITH RECURSIVE tc AS (
                SELECT r.source_id, r.target_id, 1 AS depth
                FROM relationships r WHERE r.target_id = ?1 AND r.rel_kind = 'calls'
                UNION ALL
                SELECT r.source_id, r.target_id, tc.depth + 1
                FROM relationships r JOIN tc ON r.target_id = tc.source_id
                WHERE tc.depth < ?2 AND r.rel_kind = 'calls'
            )
            SELECT DISTINCT s.id, s.name, s.kind, f.path, s.line, tc.depth
            FROM tc JOIN symbols s ON s.id = tc.source_id
            JOIN files f ON s.file_id = f.id ORDER BY tc.depth, s.name"
        )?;
        let rows = stmt.query_map(params![target_id, depth], Self::map_call_entry)?;
        let mut entries = Vec::new();
        for row in rows { entries.push(row?); }
        Ok(entries)
    }

    pub fn get_callees(&self, source_id: i64, depth: u32) -> anyhow::Result<Vec<SymbolCallEntry>> {
        if depth <= 1 { return self.get_direct_callees(source_id); }
        let mut stmt = self.conn.prepare(
            "WITH RECURSIVE tc AS (
                SELECT r.source_id, r.target_id, 1 AS depth
                FROM relationships r WHERE r.source_id = ?1 AND r.rel_kind = 'calls'
                UNION ALL
                SELECT r.source_id, r.target_id, tc.depth + 1
                FROM relationships r JOIN tc ON r.source_id = tc.target_id
                WHERE tc.depth < ?2 AND r.rel_kind = 'calls'
            )
            SELECT DISTINCT s.id, s.name, s.kind, f.path, s.line, tc.depth
            FROM tc JOIN symbols s ON s.id = tc.target_id
            JOIN files f ON s.file_id = f.id ORDER BY tc.depth, s.name"
        )?;
        let rows = stmt.query_map(params![source_id, depth], Self::map_call_entry)?;
        let mut entries = Vec::new();
        for row in rows { entries.push(row?); }
        Ok(entries)
    }

    fn get_direct_callers(&self, target_id: i64) -> anyhow::Result<Vec<SymbolCallEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT s.id, s.name, s.kind, f.path, s.line, 1 AS depth
             FROM relationships r JOIN symbols s ON s.id = r.source_id
             JOIN files f ON s.file_id = f.id
             WHERE r.target_id = ?1 AND r.rel_kind = 'calls' ORDER BY s.name"
        )?;
        let rows = stmt.query_map(params![target_id], Self::map_call_entry)?;
        let mut entries = Vec::new();
        for row in rows { entries.push(row?); }
        Ok(entries)
    }

    fn get_direct_callees(&self, source_id: i64) -> anyhow::Result<Vec<SymbolCallEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT s.id, s.name, s.kind, f.path, s.line, 1 AS depth
             FROM relationships r JOIN symbols s ON s.id = r.target_id
             JOIN files f ON s.file_id = f.id
             WHERE r.source_id = ?1 AND r.rel_kind = 'calls' ORDER BY s.name"
        )?;
        let rows = stmt.query_map(params![source_id], Self::map_call_entry)?;
        let mut entries = Vec::new();
        for row in rows { entries.push(row?); }
        Ok(entries)
    }

    fn map_call_entry(row: &rusqlite::Row) -> rusqlite::Result<SymbolCallEntry> {
        Ok(SymbolCallEntry {
            id: row.get(0)?,
            name: row.get(1)?,
            kind: serde_json::from_str(&format!("\"{}\"", row.get::<_, String>(2)?)).unwrap(),
            file_path: row.get(3)?,
            line: row.get::<_, i64>(4)? as usize,
            depth: row.get::<_, i64>(5)? as u32,
        })
    }
}
```

文件/事务操作：

```rust
impl IndexStore {
    pub fn upsert_file(&self, path: &str, sha256: &str) -> anyhow::Result<i64> {
        self.conn.execute(
            "INSERT INTO files (path, sha256) VALUES (?1, ?2)
             ON CONFLICT(path) DO UPDATE SET sha256 = excluded.sha256, indexed_at = datetime('now')",
            params![path, sha256],
        )?;
        let id: i64 = self.conn.query_row(
            "SELECT id FROM files WHERE path = ?1", params![path], |row| row.get(0),
        )?;
        Ok(id)
    }

    pub fn delete_file(&self, path: &str) -> anyhow::Result<()> {
        if let Some(file_id) = self.get_file_id(path)? {
            self.delete_symbols_for_file(file_id)?;
            self.conn.execute("DELETE FROM files WHERE id = ?1", params![file_id])?;
        }
        Ok(())
    }

    fn get_file_id(&self, path: &str) -> anyhow::Result<Option<i64>> {
        let mut stmt = self.conn.prepare("SELECT id FROM files WHERE path = ?1")?;
        let mut rows = stmt.query_map(params![path], |row| row.get(0))?;
        Ok(rows.next().transpose()?)
    }

    pub fn begin_transaction(&self) -> anyhow::Result<()> {
        self.conn.execute_batch("BEGIN TRANSACTION")?;
        Ok(())
    }

    pub fn commit(&self) -> anyhow::Result<()> {
        self.conn.execute_batch("COMMIT")?;
        Ok(())
    }

    pub fn rollback(&self) -> anyhow::Result<()> {
        self.conn.execute_batch("ROLLBACK")?;
        Ok(())
    }
}
```

- [x] **Step 3.5: 编写 IndexStore 单元测试**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::codegraph::types::*;
    use tempfile::TempDir;

    fn create_test_store() -> (IndexStore, TempDir) {
        let dir = TempDir::new().unwrap();
        let store = IndexStore::open(dir.path()).unwrap();
        (store, dir)
    }

    #[test]
    fn test_schema_creation() {
        let (store, _dir) = create_test_store();
        assert!(store.has_index().unwrap());
    }

    #[test]
    fn test_upsert_and_get_file() {
        let (store, _dir) = create_test_store();
        let file_id = store.upsert_file("src/lib.rs", "abc123").unwrap();
        assert!(file_id > 0);
        let stored_hash = store.get_file_hash("src/lib.rs").unwrap();
        assert_eq!(stored_hash, Some("abc123".to_string()));
    }

    #[test]
    fn test_symbol_crud() {
        let (store, _dir) = create_test_store();
        let file_id = store.upsert_file("src/main.rs", "abc").unwrap();
        let sym = Symbol {
            id: None, name: "main".into(), kind: SymbolKind::Function,
            file_path: "src/main.rs".into(), line: 1, col: 1,
            signature: Some("fn main()".into()), visibility: Visibility::Private,
            parent_module: None,
        };
        let sym_id = store.insert_symbol(&sym, file_id).unwrap();
        assert!(sym_id > 0);
        let found = store.get_symbol_by_id(sym_id).unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().name, "main");
    }

    #[test]
    fn test_file_deletion_removes_symbols() {
        let (store, _dir) = create_test_store();
        let file_id = store.upsert_file("src/temp.rs", "def").unwrap();
        let sym = Symbol {
            id: None, name: "temp_fn".into(), kind: SymbolKind::Function,
            file_path: "src/temp.rs".into(), line: 1, col: 1,
            signature: None, visibility: Visibility::Private, parent_module: None,
        };
        store.insert_symbol(&sym, file_id).unwrap();
        store.delete_file("src/temp.rs").unwrap();
        // verify symbols for that file are gone
        let symbols = store.get_symbols_for_file(file_id).unwrap();
        assert!(symbols.is_empty());
    }
}
```

```bash
cargo test -- tools::codegraph::store::tests 2>&1
```
预期：5 个测试通过。

- [x] **Step 3.6: 提交**

```bash
git add src/tools/codegraph/store.rs
git commit -m "feat(codegraph): implement IndexStore with SQLite schema and CRUD operations"
```

---

### Task 4: tree-sitter 解析器封装

**Files:**
- Create: `src/tools/codegraph/parser.rs`

- [ ] **Step 4.1: 实现 `CodeParser` 结构体**

文件 `src/tools/codegraph/parser.rs`：

```rust
use tree_sitter::{Parser, Language, Node};

/// Wraps a tree-sitter parser configured for Rust.
pub struct CodeParser {
    parser: Parser,
    _rust_lang: Language,
}

impl CodeParser {
    pub fn new() -> Self {
        let mut parser = Parser::new();
        let rust_lang = tree_sitter_rust::language();
        parser.set_language(rust_lang).expect("Failed to set tree-sitter Rust language");
        Self { parser, _rust_lang: rust_lang }
    }

    pub fn parse(&mut self, source: &str) -> anyhow::Result<tree_sitter::Tree> {
        let tree = self.parser.parse(source, None)
            .ok_or_else(|| anyhow::anyhow!("Failed to parse source"))?;
        Ok(tree)
    }

    pub fn root_node<'a>(&self, tree: &'a tree_sitter::Tree) -> Node<'a> {
        tree.root_node()
    }
}

impl Default for CodeParser {
    fn default() -> Self { Self::new() }
}
```

- [ ] **Step 4.2: 编写解析器单元测试**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_function_item() {
        let mut parser = CodeParser::new();
        let source = "pub fn hello(x: i32) -> bool { true }";
        let tree = parser.parse(source).unwrap();
        assert_eq!(tree.root_node().kind(), "source_file");
        assert_eq!(tree.root_node().child(0).unwrap().kind(), "function_item");
    }

    #[test]
    fn test_parse_struct_and_enum() {
        let mut parser = CodeParser::new();
        let source = "pub struct Point { x: i32, y: i32 }\nenum Color { Red, Green, Blue }";
        let tree = parser.parse(source).unwrap();
        assert_eq!(tree.root_node().child(0).unwrap().kind(), "struct_item");
        assert_eq!(tree.root_node().child(1).unwrap().kind(), "enum_item");
    }

    #[test]
    fn test_parse_with_syntax_error() {
        let mut parser = CodeParser::new();
        let source = "pub fn broken(x: i32 { ";
        let tree = parser.parse(source).unwrap();
        assert!(tree.root_node().has_error());
    }

    #[test]
    fn test_parse_empty_input() {
        let mut parser = CodeParser::new();
        let tree = parser.parse("").unwrap();
        assert_eq!(tree.root_node().kind(), "source_file");
        assert_eq!(tree.root_node().child_count(), 0);
    }
}
```

```bash
cargo test -- tools::codegraph::parser::tests 2>&1
```
预期：4 个测试通过。

- [ ] **Step 4.3: 提交**

```bash
git add src/tools/codegraph/parser.rs
git commit -m "feat(codegraph): implement tree-sitter parser wrapper for Rust"
```

---

### Task 5: 索引引擎（符号/引用/关系提取）

**Files:**
- Create: `src/tools/codegraph/indexer.rs`

- [ ] **Step 5.1: 实现 `Indexer` 结构和全量索引流程**

```rust
use crate::tools::codegraph::parser::CodeParser;
use crate::tools::codegraph::store::IndexStore;
use crate::tools::codegraph::types::*;
use std::path::Path;
use std::sync::Arc;
use std::sync::Mutex;
use walkdir::WalkDir;

/// Summary of an indexing operation.
pub struct IndexSummary {
    pub files_indexed: usize,
    pub symbols_extracted: usize,
    pub warnings: u32,
    pub elapsed_secs: f64,
}

struct FileIndexResult {
    relative_path: String,
    hash: String,
    symbols: Vec<Symbol>,
    references: Vec<Reference>,
    relationships: Vec<Relationship>,
}

/// Orchestrates file scanning, parsing, and storage for code indexing.
pub struct Indexer {
    store: Arc<IndexStore>,
    parser: Arc<Mutex<CodeParser>>,
}

impl Indexer {
    pub fn new(store: Arc<IndexStore>, parser: Arc<Mutex<CodeParser>>) -> Self {
        Self { store, parser }
    }

    pub fn index_full(&self, project_root: &Path) -> anyhow::Result<IndexSummary> {
        let files: Vec<_> = WalkDir::new(project_root)
            .follow_links(false).into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().map(|x| x == "rs").unwrap_or(false))
            .filter(|e| !e.path().to_string_lossy().contains("/target/"))
            .map(|e| e.path().to_path_buf())
            .collect();

        let mut file_data = Vec::with_capacity(files.len());
        let mut warnings = 0u32;

        for file_path in &files {
            match self.index_file(file_path, project_root) {
                Ok(result) => file_data.push(result),
                Err(e) => {
                    eprintln!("[codegraph] Warning: failed to index {}: {}", file_path.display(), e);
                    warnings += 1;
                }
            }
        }

        self.store.begin_transaction()?;
        for data in &file_data {
            let file_id = self.store.upsert_file(&data.relative_path, &data.hash)?;
            self.store.delete_symbols_for_file(file_id)?;
            for sym in &data.symbols {
                let sym_id = self.store.insert_symbol(sym, file_id)?;
                for reference in &data.references {
                    if reference.symbol_id == sym.id.unwrap_or(0) {
                        let mut r = reference.clone();
                        r.symbol_id = sym_id;
                        self.store.insert_reference(&r, file_id)?;
                    }
                }
            }
            for rel in &data.relationships {
                self.store.insert_relationship(rel)?;
            }
        }
        self.store.commit()?;

        let symbol_count: usize = file_data.iter().map(|d| d.symbols.len()).sum();
        Ok(IndexSummary { files_indexed: files.len(), symbols_extracted: symbol_count, warnings, elapsed_secs: 0.0 })
    }

    pub fn index_incremental(&self, project_root: &Path) -> anyhow::Result<IndexSummary> {
        let tracked = self.store.get_all_files()?;
        let tracked_map: std::collections::HashMap<String, String> = tracked.into_iter().collect();

        let current: Vec<_> = WalkDir::new(project_root)
            .follow_links(false).into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().map(|x| x == "rs").unwrap_or(false))
            .filter(|e| !e.path().to_string_lossy().contains("/target/"))
            .map(|e| {
                let full = e.path().to_path_buf();
                let relative = full.strip_prefix(project_root).unwrap_or(&full).to_string_lossy().to_string();
                (relative, full)
            })
            .collect();

        let mut added = 0u32;
        let mut modified = 0u32;
        let mut removed = 0u32;
        let mut warnings = 0u32;
        let mut current_set: std::collections::HashSet<String> = std::collections::HashSet::new();

        self.store.begin_transaction()?;
        for (relative, full_path) in &current {
            current_set.insert(relative.clone());
            let hash = IndexStore::file_hash(full_path)?;
            match tracked_map.get(relative) {
                None => {
                    added += 1;
                    if let Err(e) = self.index_and_store_file(full_path, project_root, &hash) {
                        eprintln!("[codegraph] Warning: {}", e); warnings += 1;
                    }
                }
                Some(old) if *old != hash => {
                    modified += 1;
                    self.store.delete_file(relative)?;
                    if let Err(e) = self.index_and_store_file(full_path, project_root, &hash) {
                        eprintln!("[codegraph] Warning: {}", e); warnings += 1;
                    }
                }
                _ => {}
            }
        }
        for (relative, _) in &tracked_map {
            if !current_set.contains(relative) {
                removed += 1;
                self.store.delete_file(relative)?;
            }
        }
        self.store.commit()?;

        Ok(IndexSummary { files_indexed: (added + modified) as usize, symbols_extracted: 0, warnings, elapsed_secs: 0.0 })
    }
}
```

- [ ] **Step 5.2: 实现符号/引用/关系提取方法**

```rust
impl Indexer {
    fn index_file(&self, file_path: &Path, project_root: &Path) -> anyhow::Result<FileIndexResult> {
        let source = std::fs::read_to_string(file_path)?;
        let hash = IndexStore::file_hash(file_path)?;
        let relative = file_path.strip_prefix(project_root).unwrap_or(file_path)
            .to_string_lossy().to_string();

        let mut parser = self.parser.lock().unwrap();
        let tree = parser.parse(&source)?;
        let root = tree.root_node();

        let mut symbols = Vec::new();
        let mut references = Vec::new();
        let mut relationships = Vec::new();

        self.extract_from_node(&root, &source, &relative, &mut symbols, &mut references, &mut relationships);
        Ok(FileIndexResult { relative_path: relative, hash, symbols, references, relationships })
    }

    fn extract_from_node(
        &self, node: &tree_sitter::Node, source: &str, file_path: &str,
        symbols: &mut Vec<Symbol>, references: &mut Vec<Reference>, relationships: &mut Vec<Relationship>,
    ) {
        let kind = node.kind();
        if let Some(sym_kind) = SymbolKind::from_node_type(kind) {
            if let Some(name_node) = self.find_name_child(node) {
                let name = self.node_text(name_node, source);
                let signature = Some(self.get_node_text(*node, source));
                let visibility = self.extract_visibility(node);
                let parent_module = self.find_parent_module(node);
                let sym_id = (symbols.len() + 1) as i64;
                symbols.push(Symbol {
                    id: Some(sym_id), name, kind: sym_kind,
                    file_path: file_path.to_string(),
                    line: node.start_position().row + 1, col: node.start_position().column + 1,
                    signature, visibility, parent_module,
                });
            }
        }
        if kind == "call_expression" {
            self.extract_call_reference(node, source, file_path, symbols, references, relationships);
        }
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.extract_from_node(child, source, file_path, symbols, references, relationships);
        }
    }

    fn find_name_child(&self, node: &tree_sitter::Node) -> Option<tree_sitter::Node> {
        // Try field name "name" first (works for most tree-sitter grammars)
        if let Ok(name_node) = node.child_by_field_name("name") {
            return Some(name_node);
        }
        // Fallback: find first identifier child
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "identifier" || child.kind() == "type_identifier" {
                return Some(child);
            }
        }
        None
    }

    fn node_text(&self, node: tree_sitter::Node, source: &str) -> String {
        source[node.start_byte()..node.end_byte()].to_string()
    }

    fn get_node_text(&self, node: tree_sitter::Node, source: &str) -> String {
        source[node.start_byte()..node.end_byte()].to_string()
    }

    fn extract_visibility(&self, node: &tree_sitter::Node) -> Visibility {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "visibility_qualifier" {
                return Visibility::from_visibility_modifier(&self.node_text(child, ""));
            }
        }
        Visibility::Private
    }

    fn find_parent_module(&self, node: &tree_sitter::Node) -> Option<String> {
        let mut parent = node.parent();
        while let Some(p) = parent {
            if p.kind() == "mod_item" {
                if let Some(name_node) = self.find_name_child(&p) {
                    return Some(self.node_text(name_node, ""));
                }
            }
            parent = p.parent();
        }
        None
    }

    fn extract_call_reference(
        &self, node: &tree_sitter::Node, source: &str, file_path: &str,
        symbols: &[Symbol], references: &mut Vec<Reference>, relationships: &mut Vec<Relationship>,
    ) {
        let children: Vec<_> = {
            let mut c = node.walk();
            node.children(&mut c).collect()
        };
        if children.is_empty() { return; }
        let func_name = self.node_text(children[0], source);
        let target_ids: Vec<i64> = symbols.iter()
            .filter(|s| s.name == func_name).map(|s| s.id.unwrap_or(0)).collect();

        references.push(Reference {
            id: Some((references.len() + 1) as i64), symbol_id: 0,
            file_path: file_path.to_string(),
            line: node.start_position().row + 1, col: node.start_position().column + 1,
            ref_kind: RefKind::Call,
            context: Some(self.get_node_text(*node, source)),
        });
        for &target_id in &target_ids {
            if let Some(source_sym) = self.find_enclosing_function(node, symbols) {
                relationships.push(Relationship {
                    id: None, source_id: source_sym.id.unwrap_or(0), target_id,
                    rel_kind: RelKind::Calls, file_path: file_path.to_string(),
                    line: node.start_position().row + 1,
                    confidence: if target_id > 0 { Confidence::High } else { Confidence::Low },
                });
            }
        }
    }

    fn find_enclosing_function<'a>(&self, node: &tree_sitter::Node<'a>, symbols: &'a [Symbol]) -> Option<&'a Symbol> {
        let mut parent = node.parent();
        while let Some(p) = parent {
            if p.kind() == "function_item" {
                if let Some(name_node) = self.find_name_child(&p) {
                    let name = self.node_text(name_node, "");
                    return symbols.iter().find(|s| s.name == name);
                }
            }
            parent = p.parent();
        }
        None
    }

    fn index_and_store_file(&self, file_path: &Path, project_root: &Path, hash: &str) -> anyhow::Result<()> {
        let relative = file_path.strip_prefix(project_root).unwrap_or(file_path)
            .to_string_lossy().to_string();
        let result = self.index_file(file_path, project_root)?;
        let file_id = self.store.upsert_file(&relative, hash)?;
        self.store.delete_symbols_for_file(file_id)?;
        for sym in &result.symbols {
            let sym_id = self.store.insert_symbol(sym, file_id)?;
            for reference in &result.references {
                if reference.symbol_id == sym.id.unwrap_or(0) {
                    let mut r = reference.clone();
                    r.symbol_id = sym_id;
                    self.store.insert_reference(&r, file_id)?;
                }
            }
        }
        for rel in &result.relationships { self.store.insert_relationship(rel)?; }
        Ok(())
    }
}
```

- [ ] **Step 5.3: 编写索引器单元测试**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::codegraph::parser::CodeParser;
    use std::sync::Mutex;
    use tempfile::TempDir;

    fn setup_indexer() -> (Indexer, TempDir) {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(IndexStore::open(dir.path()).unwrap());
        let parser = Arc::new(Mutex::new(CodeParser::new()));
        (Indexer::new(store, parser), dir)
    }

    fn write_file(dir: &Path, name: &str, content: &str) -> PathBuf {
        let path = dir.join(name);
        if let Some(parent) = path.parent() { fs::create_dir_all(parent).unwrap(); }
        fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn test_index_single_file() {
        let (indexer, dir) = setup_indexer();
        write_file(dir.path(), "lib.rs", "pub fn greet(name: &str) -> String { format!(\"Hello {}\", name) }");
        let summary = indexer.index_full(dir.path()).unwrap();
        assert_eq!(summary.files_indexed, 1);
        assert!(summary.symbols_extracted > 0);
    }

    #[test]
    fn test_index_empty_project() {
        let (indexer, dir) = setup_indexer();
        let summary = indexer.index_full(dir.path()).unwrap();
        assert_eq!(summary.files_indexed, 0);
    }

    #[test]
    fn test_incremental_index_adds_new_file() {
        let (indexer, dir) = setup_indexer();
        write_file(dir.path(), "lib.rs", "pub fn existing() {}");
        indexer.index_full(dir.path()).unwrap();
        write_file(dir.path(), "new.rs", "pub fn new_fn() {}");
        let summary = indexer.index_incremental(dir.path()).unwrap();
        assert_eq!(summary.files_indexed, 1);
    }
}
```

```bash
cargo test -- tools::codegraph::indexer::tests 2>&1
```
预期：3 个测试通过。

- [ ] **Step 5.4: 提交**

```bash
git add src/tools/codegraph/indexer.rs
git commit -m "feat(codegraph): implement full and incremental indexing with tree-sitter AST traversal"
```

---

### Task 6: 查询引擎

**Files:**
- Create: `src/tools/codegraph/query.rs`

- [ ] **Step 6.1: 实现 `QueryEngine` 和 `codegraph_node` / `codegraph_explore`**

```rust
use crate::tools::codegraph::store::{IndexStore, SymbolCallEntry};
use crate::tools::codegraph::types::*;
use std::sync::Arc;

pub struct QueryEngine {
    store: Arc<IndexStore>,
}

impl QueryEngine {
    pub fn new(store: Arc<IndexStore>) -> Self { Self { store } }

    pub fn codegraph_node(&self, symbol: &str) -> anyhow::Result<CodegraphNodeResult> {
        let symbols = self.store.get_symbol_by_name(symbol)?;
        if symbols.is_empty() {
            return Ok(CodegraphNodeResult {
                found: false, symbols: vec![], references: vec![],
                callers: vec![], callees: vec![], suggestions: vec![],
                is_entry_point: false, is_leaf: false,
            });
        }
        let mut refs = Vec::new();
        let mut callers = Vec::new();
        let mut callees = Vec::new();
        for sym in &symbols {
            if let Some(id) = sym.id {
                if let Ok(r) = self.store.get_references(id) { refs.extend(r); }
                if let Ok(c) = self.store.get_callers(id, 1) { callers.extend(c); }
                if let Ok(c) = self.store.get_callees(id, 1) { callees.extend(c); }
            }
        }
        let is_entry_point = symbols.iter().any(|s| s.name == "main");
        let is_leaf = callees.is_empty();
        Ok(CodegraphNodeResult {
            found: true, symbols, references: refs, callers, callees,
            suggestions: vec![], is_entry_point, is_leaf,
        })
    }

    pub fn codegraph_explore(&self, query: &str) -> anyhow::Result<CodegraphExploreResult> {
        let matched = self.store.get_symbol_by_name(query).unwrap_or_default();
        let mut call_graph = Vec::new();
        for sym in &matched {
            if let Some(id) = sym.id {
                if let Ok(callers) = self.store.get_callers(id, 2) {
                    call_graph.extend(callers.into_iter().map(|c| RelationEntry {
                        symbol_name: c.name, file_path: c.file_path, line: c.line,
                        relation: "caller".into(), depth: c.depth,
                    }));
                }
                if let Ok(callees) = self.store.get_callees(id, 2) {
                    call_graph.extend(callees.into_iter().map(|c| RelationEntry {
                        symbol_name: c.name, file_path: c.file_path, line: c.line,
                        relation: "callee".into(), depth: c.depth,
                    }));
                }
            }
        }
        Ok(CodegraphExploreResult { symbols: matched, call_graph })
    }

    pub fn get_callers(&self, symbol: &str, depth: u32) -> anyhow::Result<Vec<SymbolCallEntry>> {
        let depth = depth.min(5);
        let mut all = Vec::new();
        for sym in self.store.get_symbol_by_name(symbol)? {
            if let Some(id) = sym.id {
                if let Ok(c) = self.store.get_callers(id, depth) { all.extend(c); }
            }
        }
        Ok(all)
    }

    pub fn get_callees(&self, symbol: &str, depth: u32) -> anyhow::Result<Vec<SymbolCallEntry>> {
        let depth = depth.min(5);
        let mut all = Vec::new();
        for sym in self.store.get_symbol_by_name(symbol)? {
            if let Some(id) = sym.id {
                if let Ok(c) = self.store.get_callees(id, depth) { all.extend(c); }
            }
        }
        Ok(all)
    }
}
```

- [ ] **Step 6.2: 添加结果类型**

```rust
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CodegraphNodeResult {
    pub found: bool,
    pub symbols: Vec<Symbol>,
    pub references: Vec<Reference>,
    pub callers: Vec<SymbolCallEntry>,
    pub callees: Vec<SymbolCallEntry>,
    pub suggestions: Vec<SymbolSuggestion>,
    pub is_entry_point: bool,
    pub is_leaf: bool,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CodegraphExploreResult {
    pub symbols: Vec<Symbol>,
    pub call_graph: Vec<RelationEntry>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RelationEntry {
    pub symbol_name: String,
    pub file_path: String,
    pub line: usize,
    pub relation: String,
    pub depth: u32,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SymbolSuggestion {
    pub name: String,
    pub distance: usize,
}
```

- [ ] **Step 6.3: 编写查询引擎单元测试**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::codegraph::store::IndexStore;
    use crate::tools::codegraph::types::*;
    use tempfile::TempDir;

    fn setup_with_data() -> (QueryEngine, TempDir) {
        let dir = TempDir::new().unwrap();
        let store = Arc::new(IndexStore::open(dir.path()).unwrap());
        let file_id = store.upsert_file("src/lib.rs", "hash1").unwrap();

        let hello_id = store.insert_symbol(&Symbol {
            id: None, name: "hello".into(), kind: SymbolKind::Function,
            file_path: "src/lib.rs".into(), line: 1, col: 1,
            signature: Some("pub fn hello()".into()), visibility: Visibility::Pub,
            parent_module: None,
        }, file_id).unwrap();

        let world_id = store.insert_symbol(&Symbol {
            id: None, name: "world".into(), kind: SymbolKind::Function,
            file_path: "src/lib.rs".into(), line: 5, col: 1,
            signature: Some("fn world()".into()), visibility: Visibility::Private,
            parent_module: None,
        }, file_id).unwrap();

        store.insert_relationship(&Relationship {
            id: None, source_id: hello_id, target_id: world_id,
            rel_kind: RelKind::Calls, file_path: "src/lib.rs".into(),
            line: 2, confidence: Confidence::High,
        }).unwrap();

        (QueryEngine::new(store), dir)
    }

    #[test]
    fn test_codegraph_node_found() {
        let (engine, _) = setup_with_data();
        let r = engine.codegraph_node("hello").unwrap();
        assert!(r.found);
        assert_eq!(r.symbols.len(), 1);
    }

    #[test]
    fn test_codegraph_node_not_found() {
        let (engine, _) = setup_with_data();
        let r = engine.codegraph_node("nonexistent").unwrap();
        assert!(!r.found);
    }

    #[test]
    fn test_get_callers() {
        let (engine, _) = setup_with_data();
        let callers = engine.get_callers("world", 1).unwrap();
        assert_eq!(callers.len(), 1);
        assert_eq!(callers[0].name, "hello");
    }

    #[test]
    fn test_get_callees() {
        let (engine, _) = setup_with_data();
        let callees = engine.get_callees("hello", 1).unwrap();
        assert_eq!(callees.len(), 1);
        assert_eq!(callees[0].name, "world");
    }
}
```

```bash
cargo test -- tools::codegraph::query::tests 2>&1
```
预期：4 个测试通过。

- [ ] **Step 6.4: 提交**

```bash
git add src/tools/codegraph/query.rs
git commit -m "feat(codegraph): implement QueryEngine with node/explore/callers/callees"
```

---

### Task 7: 内置 Tool 实现

**Files:**
- Create: `src/tools/codegraph/tools.rs`
- Modify: `src/tools/mod.rs`（ToolRegistry 集成）
- Modify: `src/state/mod.rs`（AppState 添加 engine）

- [ ] **Step 7.1: 实现 `CodegraphNodeTool` 和 `CodegraphExploreTool`**

文件 `src/tools/codegraph/tools.rs`：

```rust
use crate::tools::codegraph::CodegraphEngine;
use crate::tools::{Tool, ToolError, ToolOutput};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct CodegraphNodeTool {
    engine: Arc<RwLock<Option<CodegraphEngine>>>,
}

impl CodegraphNodeTool {
    pub fn new(engine: Arc<RwLock<Option<CodegraphEngine>>>) -> Self {
        Self { engine }
    }
}

#[async_trait]
impl Tool for CodegraphNodeTool {
    fn name(&self) -> &str { "codegraph_node" }

    fn description(&self) -> &str {
        "Look up a symbol definition, references, callers, and callees using the codegraph index."
    }

    fn is_read_only(&self) -> bool { true }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "symbol": { "type": "string", "description": "Symbol name to look up" },
                "callers_depth": { "type": "integer", "description": "Caller query depth (default: 1, max: 5)", "default": 1, "minimum": 1, "maximum": 5 },
                "callees_depth": { "type": "integer", "description": "Callee query depth (default: 1, max: 5)", "default": 1, "minimum": 1, "maximum": 5 }
            },
            "required": ["symbol"]
        })
    }

    async fn execute(&self, input: serde_json::Value) -> Result<ToolOutput, ToolError> {
        let symbol = input["symbol"].as_str().ok_or_else(|| ToolError {
            message: "symbol is required".to_string(), code: Some("missing_parameter".to_string()),
        })?;
        let callers_depth = input["callers_depth"].as_u64().unwrap_or(1).min(5) as u32;
        let callees_depth = input["callees_depth"].as_u64().unwrap_or(1).min(5) as u32;

        let engine_guard = self.engine.read().await;
        if let Some(engine) = engine_guard.as_ref() {
            let _ = engine.refresh();
            match engine.query_engine.codegraph_node(symbol) {
                Ok(result) => {
                    let mut lines = vec![format!("[codegraph] Symbol: `{}`", symbol)];
                    if !result.found {
                        lines.push("  NOT FOUND".to_string());
                        return Ok(ToolOutput { output_type: "text".to_string(), content: lines.join("\n"), metadata: HashMap::new() });
                    }
                    for sym in &result.symbols {
                        lines.push(format!("  Kind: {} | Visibility: {}", sym.kind.as_str(), sym.visibility.as_str()));
                        lines.push(format!("  File: {}:{}:{}", sym.file_path, sym.line, sym.col));
                        if let Some(ref sig) = sym.signature { lines.push(format!("  Signature: {}", sig)); }
                        if let Some(ref p) = sym.parent_module { lines.push(format!("  Module: {}", p)); }
                    }
                    if !result.references.is_empty() {
                        lines.push(format!("\n  References ({}):", result.references.len()));
                        for r in &result.references {
                            lines.push(format!("    {}:{}:{}  {}", r.file_path, r.line, r.col, r.context.as_deref().unwrap_or("")));
                        }
                    }
                    if callers_depth > 0 {
                        if let Ok(callers) = engine.query_engine.get_callers(symbol, callers_depth) {
                            if !callers.is_empty() {
                                lines.push(format!("\n  Callers (depth={}):", callers_depth));
                                for c in &callers { lines.push(format!("    [d={}] {} ({}:{})", c.depth, c.name, c.file_path, c.line)); }
                            }
                        }
                    }
                    if callees_depth > 0 {
                        if let Ok(callees) = engine.query_engine.get_callees(symbol, callees_depth) {
                            if !callees.is_empty() {
                                lines.push(format!("\n  Callees (depth={}):", callees_depth));
                                for c in &callees { lines.push(format!("    [d={}] {} ({}:{})", c.depth, c.name, c.file_path, c.line)); }
                            }
                        }
                    }
                    return Ok(ToolOutput { output_type: "text".to_string(), content: lines.join("\n"), metadata: HashMap::new() });
                }
                Err(e) => return Err(ToolError { message: format!("codegraph query error: {}", e), code: Some("query_error".to_string()) }),
            }
        }
        Ok(ToolOutput {
            output_type: "text".to_string(),
            content: format!("[regex fallback] No codegraph index found for `{}`. Run `wgenty-code codegraph index` first.", symbol),
            metadata: HashMap::new(),
        })
    }
}

pub struct CodegraphExploreTool {
    engine: Arc<RwLock<Option<CodegraphEngine>>>,
}

impl CodegraphExploreTool {
    pub fn new(engine: Arc<RwLock<Option<CodegraphEngine>>>) -> Self { Self { engine } }
}

#[async_trait]
impl Tool for CodegraphExploreTool {
    fn name(&self) -> &str { "codegraph_explore" }

    fn description(&self) -> &str {
        "Explore the codebase by keyword. Finds matching symbols and returns related call paths."
    }

    fn is_read_only(&self) -> bool { true }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Keyword or natural language query to explore" }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, input: serde_json::Value) -> Result<ToolOutput, ToolError> {
        let query = input["query"].as_str().ok_or_else(|| ToolError {
            message: "query is required".to_string(), code: Some("missing_parameter".to_string()),
        })?;
        let engine_guard = self.engine.read().await;
        if let Some(engine) = engine_guard.as_ref() {
            let _ = engine.refresh();
            match engine.query_engine.codegraph_explore(query) {
                Ok(result) => {
                    let mut lines = vec![format!("[codegraph] Explore: \"{}\"", query)];
                    if result.symbols.is_empty() {
                        lines.push("  No symbols found.".to_string());
                        return Ok(ToolOutput { output_type: "text".to_string(), content: lines.join("\n"), metadata: HashMap::new() });
                    }
                    lines.push(format!("\n  Matched symbols ({}):", result.symbols.len()));
                    for sym in &result.symbols {
                        lines.push(format!("    {} [{}] at {}:{}", sym.name, sym.kind.as_str(), sym.file_path, sym.line));
                    }
                    if !result.call_graph.is_empty() {
                        lines.push(format!("\n  Related symbols ({}):", result.call_graph.len()));
                        for e in &result.call_graph {
                            lines.push(format!("    [{} d={}] {} ({}:{})", e.relation, e.depth, e.symbol_name, e.file_path, e.line));
                        }
                    }
                    Ok(ToolOutput { output_type: "text".to_string(), content: lines.join("\n"), metadata: HashMap::new() })
                }
                Err(e) => Err(ToolError { message: format!("codegraph explore error: {}", e), code: Some("query_error".to_string()) }),
            }
        } else {
            Ok(ToolOutput {
                output_type: "text".to_string(),
                content: format!("[regex fallback] No codegraph index for query \"{}\".", query),
                metadata: HashMap::new(),
            })
        }
    }
}
```

- [ ] **Step 7.2: 更新 `ToolRegistry` 支持 codegraph**

在 `src/tools/mod.rs` 的 `impl ToolRegistry` 中添加：

```rust
    pub fn with_codegraph(
        mut self,
        engine: std::sync::Arc<tokio::sync::RwLock<Option<crate::tools::codegraph::CodegraphEngine>>>,
    ) -> Self {
        self.register(Box::new(
            crate::tools::codegraph::tools::CodegraphNodeTool::new(engine.clone()),
        ));
        self.register(Box::new(
            crate::tools::codegraph::tools::CodegraphExploreTool::new(engine),
        ));
        self
    }
```

- [ ] **Step 7.3: 更新 `AppState` 添加 `codegraph_engine` 字段**

在 `src/state/mod.rs`：

```rust
    pub codegraph_engine: Arc<RwLock<Option<crate::tools::codegraph::CodegraphEngine>>>,
```

在 `AppState::new()` 中添加初始化：
```rust
    codegraph_engine: Arc::new(RwLock::new(None)),
```

在 `impl Default for AppState` 中也添加：
```rust
    codegraph_engine: Arc::new(RwLock::new(None)),
```

更新 `ToolRegistryState::default()`：
```rust
    let registry =
        crate::tools::ToolRegistry::new()
            .with_settings(&crate::config::Settings::default())
            .with_codegraph(Arc::new(RwLock::new(None)));
```

- [ ] **Step 7.4: 验证编译**

```bash
cargo check 2>&1
```
预期：编译通过（engine 为 None 时工具会输出 fallback 提示）。

- [ ] **Step 7.5: 提交**

```bash
git add src/tools/codegraph/tools.rs src/tools/mod.rs src/state/mod.rs
git commit -m "feat(codegraph): implement CodegraphNodeTool and CodegraphExploreTool, wire into ToolRegistry"
```

---

### Task 8: CLI 子命令（codegraph index/query/clean）

**Files:**
- Modify: `src/cli/mod.rs`（添加 `CodegraphCommands` 枚举）
- Modify: `src/cli/args.rs`（添加 `run_codegraph` 处理）

- [ ] **Step 8.1: 在 `Commands` 枚举中添加 `Codegraph` 变体**

在 `src/cli/mod.rs` 中添加（在 Sandbox 变体后面）：

```rust
    /// Manage codegraph code index
    Codegraph {
        #[command(subcommand)]
        action: CodegraphCommands,
    },
```

定义子命令枚举：
```rust
#[derive(Subcommand, Debug)]
pub enum CodegraphCommands {
    /// Build or update the codegraph index
    Index {
        /// Force full re-index (default: incremental)
        #[arg(long, short)]
        force: bool,
    },
    /// Query a symbol in the codegraph index
    Query {
        /// Symbol name to look up
        symbol: String,
    },
    /// Remove the codegraph index
    Clean,
}
```

- [ ] **Step 8.2: 在 `args.rs` 中添加 `run_codegraph` 处理**

在 `run_async` 中添加 match arm（Sandbox 后面）：
```rust
            Some(super::Commands::Codegraph { action }) => {
                self.run_codegraph(state, action).await?;
            }
```

添加处理方法：
```rust
    async fn run_codegraph(
        &self,
        state: crate::state::AppState,
        action: &super::CodegraphCommands,
    ) -> anyhow::Result<()> {
        use crate::tools::codegraph::CodegraphEngine;

        let project_root = self.path.clone().unwrap_or_else(|| std::env::current_dir().unwrap());

        match action {
            super::CodegraphCommands::Index { force } => {
                println!("CodeGraph Index");
                println!("  Project root: {}", project_root.display());
                let start = std::time::Instant::now();
                let engine = CodegraphEngine::new(project_root.clone(), false)?;

                if *force {
                    let db_path = project_root.join(".codegraph").join("index.db");
                    if db_path.exists() { std::fs::remove_file(&db_path)?; }
                }

                let summary = if engine.has_index() && !*force {
                    println!("  Performing incremental update...");
                    engine.indexer.index_incremental(&project_root)?
                } else {
                    println!("  Performing full indexing...");
                    engine.indexer.index_full(&project_root)?
                };

                let elapsed = start.elapsed();
                println!();
                println!("  Indexing complete");
                println!("     Files indexed: {}", summary.files_indexed);
                println!("     Symbols extracted: {}", summary.symbols_extracted);
                println!("     Warnings: {}", summary.warnings);
                println!("     Elapsed: {:.2}s", elapsed.as_secs_f64());
            }

            super::CodegraphCommands::Query { symbol } => {
                let engine = CodegraphEngine::new(project_root, true)?;
                match engine.query_engine.codegraph_node(symbol) {
                    Ok(result) => {
                        if !result.found {
                            println!("Symbol not found: {}", symbol);
                            return Ok(());
                        }
                        for sym in &result.symbols {
                            println!("{}", sym.name);
                            println!("  Kind: {} | Visibility: {}", sym.kind.as_str(), sym.visibility.as_str());
                            println!("  File: {}:{}:{}", sym.file_path, sym.line, sym.col);
                            if let Some(ref sig) = sym.signature { println!("  Signature: {}", sig); }
                        }
                        if !result.references.is_empty() {
                            println!("\n  References:");
                            for r in &result.references {
                                println!("    {}:{}:{}  {}", r.file_path, r.line, r.col, r.context.as_deref().unwrap_or(""));
                            }
                        }
                        if !result.callers.is_empty() {
                            println!("\n  Callers:");
                            for c in &result.callers { println!("    [d={}] {} ({}:{})", c.depth, c.name, c.file_path, c.line); }
                        }
                        if !result.callees.is_empty() {
                            println!("\n  Callees:");
                            for c in &result.callees { println!("    [d={}] {} ({}:{})", c.depth, c.name, c.file_path, c.line); }
                        }
                    }
                    Err(e) => eprintln!("Error querying codegraph: {}", e),
                }
            }

            super::CodegraphCommands::Clean => {
                let codegraph_dir = project_root.join(".codegraph");
                if codegraph_dir.exists() {
                    std::fs::remove_dir_all(&codegraph_dir)?;
                    println!("CodeGraph index removed: {}", codegraph_dir.display());
                } else {
                    println!("No codegraph index found at {}", codegraph_dir.display());
                }
            }
        }
        Ok(())
    }
```

- [ ] **Step 8.3: 验证编译**

```bash
cargo check 2>&1
```
预期：编译通过。

- [ ] **Step 8.4: 提交**

```bash
git add src/cli/mod.rs src/cli/args.rs
git commit -m "feat(codegraph): add CLI subcommands index/query/clean"
```

---

### Task 9: MCP 自动集成

**Files:**
- Modify: `src/mcp/tools.rs`（添加测试验证）

- [ ] **Step 9.1: 验证 MCP 自动注册**

不修改代码 -- `register_builtin_tools()` 已经遍历 `self.local_registry.list()`，新注册的 codegraph 工具会自动出现。

在 `src/mcp/tools.rs` 添加测试验证：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_register_builtin_tools_includes_codegraph() {
        let registry = ToolRegistry::new();
        registry.register_builtin_tools().await;
        let tools = registry.list().await;
        let names: Vec<String> = tools.into_iter().map(|t| t.name).collect();
        assert!(names.contains(&"codegraph_node".to_string()));
        assert!(names.contains(&"codegraph_explore".to_string()));
    }
}
```

```bash
cargo test -- mcp::tools::tests 2>&1
```
预期：通过。

- [ ] **Step 9.2: 提交**

```bash
git add src/mcp/tools.rs
git commit -m "feat(codegraph): verify MCP auto-registration of codegraph tools"
```

---

### Task 10: 与 lsp.rs 集成（优先使用索引）

**Files:**
- Modify: `src/tools/meta/lsp.rs`

- [ ] **Step 10.1: 标记 regex fallback 输出**

修改 `format_results` 函数，在输出前添加 `[regex fallback]` 前缀：

原始代码：
```rust
    let mut lines = vec![format!(
        "Found {} {} for `{}`:\n",
        results.len().min(max),
        label,
        symbol
    )];
```

修改为：
```rust
    let mut lines = vec![format!(
        "[regex fallback] Found {} {} for `{}`:\n",
        results.len().min(max),
        label,
        symbol
    )];
```

同时在 metadata 中添加 source 字段：
```rust
    metadata.insert("source".to_string(), serde_json::json!("regex_fallback"));
```

- [ ] **Step 10.2: 验证编译**

```bash
cargo check 2>&1
```
预期：编译通过。

- [ ] **Step 10.3: 提交**

```bash
git add src/tools/meta/lsp.rs
git commit -m "feat(codegraph): mark lsp tool output as regex fallback source"
```

---

### Task 11: 测试 -- 单元测试和集成测试

**Files:**
- Create: `tests/codegraph_integration.rs`

- [ ] **Step 11.1: 全量索引 + 查询集成测试**

```rust
#[cfg(test)]
mod tests {
    use std::path::Path;

    #[test]
    fn test_full_index_and_query() {
        use wgenty_code::tools::codegraph::CodegraphEngine;

        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();

        std::fs::write(root.join("lib.rs"),
            "pub fn greet(name: &str) -> String { format!(\"Hello {}\", name) }\n\
             pub fn farewell(name: &str) -> String { format!(\"Goodbye {}\", name) }\n"
        ).unwrap();
        std::fs::write(root.join("main.rs"),
            "mod lib;\nfn main() {\n    let msg = lib::greet(\"World\");\n    println!(\"{}\", msg);\n}\n"
        ).unwrap();

        let engine = CodegraphEngine::new(root.to_path_buf(), true).unwrap();
        assert!(engine.has_index());

        let result = engine.query_engine.codegraph_node("greet").unwrap();
        assert!(result.found);
        assert_eq!(result.symbols[0].name, "greet");
    }
}
```

- [ ] **Step 11.2: 增量索引测试**

```rust
    #[test]
    fn test_incremental_index_after_modify() {
        use wgenty_code::tools::codegraph::CodegraphEngine;

        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();

        std::fs::write(root.join("lib.rs"), "pub fn foo() {}").unwrap();
        let engine = CodegraphEngine::new(root.to_path_buf(), true).unwrap();
        assert!(engine.query_engine.codegraph_node("foo").unwrap().found);

        std::fs::write(root.join("lib.rs"), "pub fn bar() {}").unwrap();
        engine.refresh().unwrap();

        assert!(!engine.query_engine.codegraph_node("foo").unwrap().found);
        assert!(engine.query_engine.codegraph_node("bar").unwrap().found);
    }
```

- [ ] **Step 11.3: 调用图深度测试**

```rust
    #[test]
    fn test_call_graph_depth() {
        use wgenty_code::tools::codegraph::CodegraphEngine;

        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();

        std::fs::write(root.join("chain.rs"),
            "fn a() { b(); }\nfn b() { c(); }\nfn c() { d(); }\nfn d() {}\n"
        ).unwrap();

        let engine = CodegraphEngine::new(root.to_path_buf(), true).unwrap();

        let callers_d1 = engine.query_engine.get_callers("d", 1).unwrap();
        assert_eq!(callers_d1.len(), 1);

        let callers_d3 = engine.query_engine.get_callers("d", 3).unwrap();
        assert_eq!(callers_d3.len(), 3);

        let callees_a3 = engine.query_engine.get_callees("a", 3).unwrap();
        assert_eq!(callees_a3.len(), 3);
    }
```

- [ ] **Step 11.4: 运行完整测试套件**

```bash
cargo test --all 2>&1
```
预期：全部通过。

```bash
cargo clippy --all-targets -- -D warnings 2>&1
cargo fmt -- --check 2>&1
```
预期：均无错误。

- [ ] **Step 11.5: 提交**

```bash
git add tests/codegraph_integration.rs
git commit -m "test(codegraph): add integration tests for index, query, and call graph depth"
```

---

### Task 12: 收尾 -- .gitignore、CLAUDE.md 和最终验证

**Files:**
- Modify: `.gitignore`
- Modify: `CLAUDE.md`

- [ ] **Step 12.1: 将 `.codegraph/` 添加到 `.gitignore`**

在 `.gitignore` 末尾追加：
```
# CodeGraph index
.codegraph/
```

- [ ] **Step 12.2: 更新 `CLAUDE.md`**

在 `Architecture` 部分添加 CodeGraph 模块说明：

````markdown
  CodeGraph:
    src/tools/codegraph/  -- tree-sitter-based persistent code index
      types.rs            -- core types (Symbol, SymbolKind, Reference, etc.)
      store.rs            -- SQLite-backed IndexStore
      parser.rs           -- tree-sitter Rust parser wrapper
      indexer.rs          -- full/incremental indexing
      query.rs            -- QueryEngine (codegraph_node, codegraph_explore, call graph)
      tools.rs            -- Tool trait impls (CodegraphNodeTool, CodegraphExploreTool)

## CodeGraph

In repositories indexed by CodeGraph (a `.codegraph/` directory exists at the repo root),
reach for it BEFORE grep/find or reading files when you need to understand or locate code:

- **Built-in tools**: `codegraph_explore` answers most code questions in one call.
  `codegraph_node` returns one symbol's definition + callers/callees.
- **CLI**: `wgenty-code codegraph index` to build/update the index,
  `wgenty-code codegraph query <symbol>` for quick lookups.
- **Lazy initialization**: The engine is created automatically on first query.
  Run `wgenty-code codegraph clean` to remove the index.
````

- [ ] **Step 12.3: 最终端到端验证**

```bash
cargo build 2>&1 && cargo test --all 2>&1 && cargo clippy --all-targets -- -D warnings 2>&1 && cargo fmt -- --check 2>&1
```
全部通过。

- [ ] **Step 12.4: 最终提交**

```bash
git add .gitignore CLAUDE.md
git commit -m "chore(codegraph): add .codegraph to gitignore and update CLAUDE.md"
```

---

## 自审检查清单

**1. 规格覆盖检查：**
- [x] Task 2 — 数据类型（SymbolKind 映射、枚举定义）
- [x] Task 3 — SQLite schema（files/symbols/refs/relationships 表）
- [x] Task 4 — tree-sitter 解析器初始化和 AST 遍历
- [x] Task 5 — 全量索引（扫描→解析→提取→存储）、增量索引（哈希比对）、错误容错
- [x] Task 6 — codegraph_node/codegraph_explore/get_callers/get_callees（递归 CTE 传递闭包）
- [x] Task 7 — 内置 Tool trait 实现（read-only）
- [x] Task 8 — CLI 子命令（index/query/clean）
- [x] Task 9 — MCP 通过现有 `register_builtin_tools()` 自动暴露
- [x] Task 10 — 索引优先策略（标记 regex fallback 来源）
- [x] Task 11 — 单元测试 + 集成测试
- [x] Task 12 — .gitignore + CLAUDE.md 更新

**2. 占位符扫描：** 无 TBD、TODO 或 "实现细节后续补充" 等占位符。

**3. 类型一致性检查：** 所有类型（Symbol、SymbolKind、Visibility、Reference、Relationship、RelKind、Confidence）在 Task 2 定义，Task 3-7 一致引用。`SymbolCallEntry` 在 store.rs 定义、query.rs 使用。`CodegraphNodeResult`/`CodegraphExploreResult` 在 query.rs 定义、tools.rs 使用。

---

## 执行交接

计划完成并保存至 `docs/superpowers/plans/2026-06-13-code-graph-tool.md`。

两种执行方式：

1. **Subagent-Driven（推荐）** — 每个 task 分派独立 subagent，审查轮次间快速迭代
2. **Inline Execution** — 在当前会话中使用 executing-plans 技能执行，批处理 + 检查点

请选择执行方式？
