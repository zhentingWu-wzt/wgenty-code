use crate::tools::codegraph::types::{
    Confidence, RefKind, RelKind, Reference, Relationship, Symbol, SymbolKind,
};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
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
        let store = Self { conn, db_path };
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
        if !self.db_path.exists() {
            return Ok(false);
        }
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM files", [], |row| row.get(0))?;
        Ok(count > 0)
    }

    // ── File operations ──

    pub fn file_hash(path: &Path) -> anyhow::Result<String> {
        let contents = fs::read(path)?;
        let mut hasher = Sha256::new();
        hasher.update(&contents);
        Ok(format!("{:x}", hasher.finalize()))
    }

    pub fn get_file_hash(&self, path: &str) -> anyhow::Result<Option<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT sha256 FROM files WHERE path = ?1")?;
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
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    pub fn upsert_file(&self, path: &str, sha256: &str) -> anyhow::Result<i64> {
        self.conn.execute(
            "INSERT INTO files (path, sha256) VALUES (?1, ?2)
             ON CONFLICT(path) DO UPDATE SET sha256 = excluded.sha256, indexed_at = datetime('now')",
            params![path, sha256],
        )?;
        let id: i64 = self.conn.query_row(
            "SELECT id FROM files WHERE path = ?1",
            params![path],
            |row| row.get(0),
        )?;
        Ok(id)
    }

    pub fn delete_file(&self, path: &str) -> anyhow::Result<()> {
        if let Some(file_id) = self.get_file_id(path)? {
            self.delete_symbols_for_file(file_id)?;
            self.conn
                .execute("DELETE FROM files WHERE id = ?1", params![file_id])?;
        }
        Ok(())
    }

    fn get_file_id(&self, path: &str) -> anyhow::Result<Option<i64>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id FROM files WHERE path = ?1")?;
        let mut rows = stmt.query_map(params![path], |row| row.get(0))?;
        Ok(rows.next().transpose()?)
    }

    // ── Symbol CRUD ──

    pub fn insert_symbol(&self, sym: &Symbol, file_id: i64) -> anyhow::Result<i64> {
        self.conn.execute(
            "INSERT INTO symbols (name, kind, file_id, line, col, signature, visibility, parent_module)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                sym.name,
                sym.kind.as_str(),
                file_id,
                sym.line as i64,
                sym.col as i64,
                sym.signature,
                sym.visibility.as_str(),
                sym.parent_module,
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn delete_symbols_for_file(&self, file_id: i64) -> anyhow::Result<()> {
        self.conn
            .execute("DELETE FROM refs WHERE file_id = ?1", params![file_id])?;
        self.conn.execute(
            "DELETE FROM relationships WHERE file_id = ?1",
            params![file_id],
        )?;
        self.conn
            .execute("DELETE FROM symbols WHERE file_id = ?1", params![file_id])?;
        Ok(())
    }

    pub fn get_symbol_by_name(&self, name: &str) -> anyhow::Result<Vec<Symbol>> {
        let mut stmt = self.conn.prepare(
            "SELECT s.id, s.name, s.kind, f.path, s.line, s.col, s.signature, s.visibility, s.parent_module
             FROM symbols s JOIN files f ON s.file_id = f.id
             WHERE s.name = ?1 ORDER BY f.path, s.line",
        )?;
        let rows = stmt.query_map(params![name], Self::map_symbol)?;
        let mut symbols = Vec::new();
        for row in rows {
            symbols.push(row?);
        }
        Ok(symbols)
    }

    pub fn get_symbol_by_id(&self, id: i64) -> anyhow::Result<Option<Symbol>> {
        let mut stmt = self.conn.prepare(
            "SELECT s.id, s.name, s.kind, f.path, s.line, s.col, s.signature, s.visibility, s.parent_module
             FROM symbols s JOIN files f ON s.file_id = f.id WHERE s.id = ?1",
        )?;
        let mut rows = stmt.query_map(params![id], Self::map_symbol)?;
        Ok(rows.next().transpose()?)
    }

    pub fn get_symbols_for_file(&self, file_id: i64) -> anyhow::Result<Vec<Symbol>> {
        let mut stmt = self.conn.prepare(
            "SELECT s.id, s.name, s.kind, f.path, s.line, s.col, s.signature, s.visibility, s.parent_module
             FROM symbols s JOIN files f ON s.file_id = f.id
             WHERE s.file_id = ?1 ORDER BY s.line",
        )?;
        let rows = stmt.query_map(params![file_id], Self::map_symbol)?;
        let mut symbols = Vec::new();
        for row in rows {
            symbols.push(row?);
        }
        Ok(symbols)
    }

    fn map_symbol(row: &rusqlite::Row) -> rusqlite::Result<Symbol> {
        let kind_str: String = row.get(2)?;
        Ok(Symbol {
            id: Some(row.get(0)?),
            name: row.get(1)?,
            kind: SymbolKind::from_node_type(&kind_str).unwrap_or(SymbolKind::Function),
            file_path: row.get(3)?,
            line: row.get::<_, i64>(4)? as usize,
            col: row.get::<_, i64>(5)? as usize,
            signature: row.get(6)?,
            visibility: {
                let v: String = row.get(7)?;
                match v.as_str() {
                    "pub" => crate::tools::codegraph::types::Visibility::Pub,
                    "pub(crate)" => crate::tools::codegraph::types::Visibility::PubCrate,
                    "pub(super)" => crate::tools::codegraph::types::Visibility::PubSuper,
                    _ => crate::tools::codegraph::types::Visibility::Private,
                }
            },
            parent_module: row.get(8)?,
        })
    }

    // ── Reference CRUD ──

    pub fn insert_reference(&self, reference: &Reference, file_id: i64) -> anyhow::Result<i64> {
        self.conn.execute(
            "INSERT INTO refs (symbol_id, file_id, line, col, ref_kind, context)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                reference.symbol_id,
                file_id,
                reference.line as i64,
                reference.col as i64,
                reference.ref_kind.as_str(),
                reference.context,
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn get_references(&self, symbol_id: i64) -> anyhow::Result<Vec<Reference>> {
        let mut stmt = self.conn.prepare(
            "SELECT r.id, r.symbol_id, f.path, r.line, r.col, r.ref_kind, r.context
             FROM refs r JOIN files f ON r.file_id = f.id
             WHERE r.symbol_id = ?1 ORDER BY f.path, r.line",
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
        for row in rows {
            refs.push(row?);
        }
        Ok(refs)
    }

    // ── Relationship CRUD ──

    pub fn insert_relationship(&self, rel: &Relationship) -> anyhow::Result<i64> {
        let file_id = self
            .get_file_id(&rel.file_path)?
            .unwrap_or(0);
        self.conn.execute(
            "INSERT INTO relationships (source_id, target_id, rel_kind, file_id, line, confidence)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                rel.source_id,
                rel.target_id,
                rel.rel_kind.as_str(),
                file_id,
                rel.line as i64,
                rel.confidence.as_str(),
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    // ── Transitive closure (call graph) ──

    #[allow(dead_code)]
    pub fn get_callers(&self, target_id: i64, depth: u32) -> anyhow::Result<Vec<SymbolCallEntry>> {
        if depth <= 1 {
            return self.get_direct_callers(target_id);
        }
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
            JOIN files f ON s.file_id = f.id ORDER BY tc.depth, s.name",
        )?;
        let rows = stmt.query_map(params![target_id, depth], Self::map_call_entry)?;
        let mut entries = Vec::new();
        for row in rows {
            entries.push(row?);
        }
        Ok(entries)
    }

    #[allow(dead_code)]
    pub fn get_callees(
        &self,
        source_id: i64,
        depth: u32,
    ) -> anyhow::Result<Vec<SymbolCallEntry>> {
        if depth <= 1 {
            return self.get_direct_callees(source_id);
        }
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
            JOIN files f ON s.file_id = f.id ORDER BY tc.depth, s.name",
        )?;
        let rows = stmt.query_map(params![source_id, depth], Self::map_call_entry)?;
        let mut entries = Vec::new();
        for row in rows {
            entries.push(row?);
        }
        Ok(entries)
    }

    fn get_direct_callers(&self, target_id: i64) -> anyhow::Result<Vec<SymbolCallEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT s.id, s.name, s.kind, f.path, s.line, 1 AS depth
             FROM relationships r JOIN symbols s ON s.id = r.source_id
             JOIN files f ON s.file_id = f.id
             WHERE r.target_id = ?1 AND r.rel_kind = 'calls' ORDER BY s.name",
        )?;
        let rows = stmt.query_map(params![target_id], Self::map_call_entry)?;
        let mut entries = Vec::new();
        for row in rows {
            entries.push(row?);
        }
        Ok(entries)
    }

    fn get_direct_callees(&self, source_id: i64) -> anyhow::Result<Vec<SymbolCallEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT s.id, s.name, s.kind, f.path, s.line, 1 AS depth
             FROM relationships r JOIN symbols s ON s.id = r.target_id
             JOIN files f ON s.file_id = f.id
             WHERE r.source_id = ?1 AND r.rel_kind = 'calls' ORDER BY s.name",
        )?;
        let rows = stmt.query_map(params![source_id], Self::map_call_entry)?;
        let mut entries = Vec::new();
        for row in rows {
            entries.push(row?);
        }
        Ok(entries)
    }

    fn map_call_entry(row: &rusqlite::Row) -> rusqlite::Result<SymbolCallEntry> {
        let kind_str: String = row.get(2)?;
        Ok(SymbolCallEntry {
            id: row.get(0)?,
            name: row.get(1)?,
            kind: SymbolKind::from_node_type(&kind_str).unwrap_or(SymbolKind::Function),
            file_path: row.get(3)?,
            line: row.get::<_, i64>(4)? as usize,
            depth: row.get::<_, i64>(5)? as u32,
        })
    }

    // ── Transactions ──

    pub fn begin_transaction(&self) -> anyhow::Result<()> {
        self.conn.execute_batch("BEGIN TRANSACTION")?;
        Ok(())
    }

    pub fn commit(&self) -> anyhow::Result<()> {
        self.conn.execute_batch("COMMIT")?;
        Ok(())
    }

    #[allow(dead_code)]
    pub fn rollback(&self) -> anyhow::Result<()> {
        self.conn.execute_batch("ROLLBACK")?;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SymbolCallEntry {
    pub id: i64,
    pub name: String,
    pub kind: SymbolKind,
    pub file_path: String,
    pub line: usize,
    pub depth: u32,
}

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
        assert!(!store.has_index().unwrap()); // empty schema, no files yet
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
            id: None,
            name: "main".into(),
            kind: SymbolKind::Function,
            file_path: "src/main.rs".into(),
            line: 1,
            col: 1,
            signature: Some("fn main()".into()),
            visibility: Visibility::Private,
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
            id: None,
            name: "temp_fn".into(),
            kind: SymbolKind::Function,
            file_path: "src/temp.rs".into(),
            line: 1,
            col: 1,
            signature: None,
            visibility: Visibility::Private,
            parent_module: None,
        };
        store.insert_symbol(&sym, file_id).unwrap();
        store.delete_file("src/temp.rs").unwrap();
        let symbols = store.get_symbols_for_file(file_id).unwrap();
        assert!(symbols.is_empty());
    }

    #[test]
    fn test_compute_file_hash() {
        // verify SHA256 is deterministic
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.txt");
        std::fs::write(&path, b"hello").unwrap();
        let hash1 = IndexStore::file_hash(&path).unwrap();
        let hash2 = IndexStore::file_hash(&path).unwrap();
        assert_eq!(hash1, hash2);
        assert_eq!(hash1.len(), 64);
    }
}
