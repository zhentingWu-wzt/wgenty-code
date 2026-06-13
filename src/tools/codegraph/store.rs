use crate::tools::codegraph::types::{RefKind, Reference, Relationship, Symbol, SymbolKind};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::fs;

pub struct IndexStore {
    conn: std::sync::Mutex<Connection>,
    db_path: PathBuf,
}

impl IndexStore {
    pub fn open(project_root: &Path) -> anyhow::Result<Self> {
        let codegraph_dir = project_root.join(".codegraph");
        fs::create_dir_all(&codegraph_dir)?;
        let db_path = codegraph_dir.join("index.db");
        let conn = std::sync::Mutex::new(Connection::open(&db_path)?);
        conn.lock().unwrap().execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        let store = Self { conn, db_path };
        store.create_schema()?;
        Ok(store)
    }

    fn create_schema(&self) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(
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
        let conn = self.conn.lock().unwrap();
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM files", [], |row| row.get(0))?;
        Ok(count > 0)
    }

    // ── File ──

    pub fn file_hash(path: &Path) -> anyhow::Result<String> {
        let contents = fs::read(path)?;
        let mut hasher = Sha256::new();
        hasher.update(&contents);
        Ok(format!("{:x}", hasher.finalize()))
    }

    pub fn get_file_hash(&self, path: &str) -> anyhow::Result<Option<String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT sha256 FROM files WHERE path = ?1")?;
        let mut rows = stmt.query_map(params![path], |row| row.get(0))?;
        match rows.next() { Some(Ok(h)) => Ok(Some(h)), _ => Ok(None) }
    }

    pub fn get_all_files(&self) -> anyhow::Result<Vec<(String, String)>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT path, sha256 FROM files")?;
        let rows = stmt.query_map([], |row| Ok((row.get::<_,String>(0)?, row.get::<_,String>(1)?)))?;
        let mut r = Vec::new(); for row in rows { r.push(row?); } Ok(r)
    }

    pub fn upsert_file(&self, path: &str, sha256: &str) -> anyhow::Result<i64> {
        let conn = self.conn.lock().unwrap();
        conn.execute("INSERT INTO files (path, sha256) VALUES (?1,?2) ON CONFLICT(path) DO UPDATE SET sha256=excluded.sha256, indexed_at=datetime('now')", params![path, sha256])?;
        let id: i64 = conn.query_row("SELECT id FROM files WHERE path=?1", params![path], |row| row.get(0))?;
        Ok(id)
    }

    pub fn delete_file(&self, path: &str) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        let mut s = conn.prepare("SELECT id FROM files WHERE path=?1")?;
        let rows = s.query_map(params![path], |r| r.get::<_, i64>(0))?;
        let fid = rows.into_iter().next().transpose()?;
        drop(s);
        if let Some(fid) = fid {
            conn.execute("DELETE FROM refs WHERE file_id=?1", params![fid])?;
            conn.execute("DELETE FROM relationships WHERE file_id=?1", params![fid])?;
            conn.execute("DELETE FROM symbols WHERE file_id=?1", params![fid])?;
            conn.execute("DELETE FROM files WHERE id=?1", params![fid])?;
        }
        Ok(())
    }

    // ── Symbol ──

    pub fn insert_symbol(&self, sym: &Symbol, file_id: i64) -> anyhow::Result<i64> {
        let conn = self.conn.lock().unwrap();
        conn.execute("INSERT INTO symbols (name,kind,file_id,line,col,signature,visibility,parent_module) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
            params![sym.name, sym.kind.as_str(), file_id, sym.line as i64, sym.col as i64, sym.signature, sym.visibility.as_str(), sym.parent_module])?;
        Ok(conn.last_insert_rowid())
    }

    pub fn delete_symbols_for_file(&self, file_id: i64) -> anyhow::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM refs WHERE file_id=?1", params![file_id])?;
        conn.execute("DELETE FROM relationships WHERE file_id=?1", params![file_id])?;
        conn.execute("DELETE FROM symbols WHERE file_id=?1", params![file_id])?;
        Ok(())
    }

    pub fn get_symbol_by_name(&self, name: &str) -> anyhow::Result<Vec<Symbol>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT s.id,s.name,s.kind,f.path,s.line,s.col,s.signature,s.visibility,s.parent_module FROM symbols s JOIN files f ON s.file_id=f.id WHERE s.name=?1 ORDER BY f.path,s.line")?;
        let rows = stmt.query_map(params![name], Self::map_symbol)?;
        let mut syms = Vec::new(); for r in rows { syms.push(r?); } Ok(syms)
    }

    pub fn get_symbol_by_id(&self, id: i64) -> anyhow::Result<Option<Symbol>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT s.id,s.name,s.kind,f.path,s.line,s.col,s.signature,s.visibility,s.parent_module FROM symbols s JOIN files f ON s.file_id=f.id WHERE s.id=?1")?;
        let mut rows = stmt.query_map(params![id], Self::map_symbol)?;
        Ok(rows.next().transpose()?)
    }

    pub fn get_symbols_for_file(&self, file_id: i64) -> anyhow::Result<Vec<Symbol>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT s.id,s.name,s.kind,f.path,s.line,s.col,s.signature,s.visibility,s.parent_module FROM symbols s JOIN files f ON s.file_id=f.id WHERE s.file_id=?1 ORDER BY s.line")?;
        let rows = stmt.query_map(params![file_id], Self::map_symbol)?;
        let mut syms = Vec::new(); for r in rows { syms.push(r?); } Ok(syms)
    }

    fn map_symbol(row: &rusqlite::Row) -> rusqlite::Result<Symbol> {
        let v: String = row.get(7)?;
        Ok(Symbol {
            id: Some(row.get(0)?), name: row.get(1)?,
            kind: SymbolKind::from_node_type(&row.get::<_,String>(2)?).unwrap_or(SymbolKind::Function),
            file_path: row.get(3)?, line: row.get::<_,i64>(4)? as usize,
            col: row.get::<_,i64>(5)? as usize, signature: row.get(6)?,
            visibility: match v.as_str() {
                "pub" => crate::tools::codegraph::types::Visibility::Pub,
                "pub(crate)" => crate::tools::codegraph::types::Visibility::PubCrate,
                "pub(super)" => crate::tools::codegraph::types::Visibility::PubSuper,
                _ => crate::tools::codegraph::types::Visibility::Private,
            },
            parent_module: row.get(8)?,
        })
    }

    // ── Reference ──

    pub fn insert_reference(&self, reference: &Reference, file_id: i64) -> anyhow::Result<i64> {
        let conn = self.conn.lock().unwrap();
        conn.execute("INSERT INTO refs (symbol_id,file_id,line,col,ref_kind,context) VALUES (?1,?2,?3,?4,?5,?6)",
            params![reference.symbol_id, file_id, reference.line as i64, reference.col as i64, reference.ref_kind.as_str(), reference.context])?;
        Ok(conn.last_insert_rowid())
    }

    pub fn get_references(&self, symbol_id: i64) -> anyhow::Result<Vec<Reference>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT r.id,r.symbol_id,f.path,r.line,r.col,r.ref_kind,r.context FROM refs r JOIN files f ON r.file_id=f.id WHERE r.symbol_id=?1 ORDER BY f.path,r.line")?;
        let rows = stmt.query_map(params![symbol_id], |row| Ok(Reference {
            id: Some(row.get(0)?), symbol_id: row.get(1)?, file_path: row.get(2)?,
            line: row.get::<_,i64>(3)? as usize, col: row.get::<_,i64>(4)? as usize,
            ref_kind: RefKind::from_str(&row.get::<_,String>(5)?), context: row.get(6)?,
        }))?;
        let mut refs = Vec::new(); for r in rows { refs.push(r?); } Ok(refs)
    }

    // ── Relationship ──

    pub fn insert_relationship(&self, rel: &Relationship) -> anyhow::Result<i64> {
        let conn = self.conn.lock().unwrap();
        let mut s = conn.prepare("SELECT id FROM files WHERE path=?1")?;
        let rows = s.query_map(params![&rel.file_path], |r| r.get::<_, i64>(0))?;
        let fid = rows.into_iter().next().transpose()?.unwrap_or(0);
        drop(s);
        conn.execute("INSERT INTO relationships (source_id,target_id,rel_kind,file_id,line,confidence) VALUES (?1,?2,?3,?4,?5,?6)",
            params![rel.source_id, rel.target_id, rel.rel_kind.as_str(), fid, rel.line as i64, rel.confidence.as_str()])?;
        Ok(conn.last_insert_rowid())
    }

    // ── Call graph ──

    pub fn get_callers(&self, target_id: i64, depth: u32) -> anyhow::Result<Vec<SymbolCallEntry>> {
        let conn = self.conn.lock().unwrap();
        if depth <= 1 {
            let mut stmt = conn.prepare("SELECT s.id,s.name,s.kind,f.path,s.line,1 AS depth FROM relationships r JOIN symbols s ON s.id=r.source_id JOIN files f ON s.file_id=f.id WHERE r.target_id=?1 AND r.rel_kind='calls' ORDER BY s.name")?;
            let rows = stmt.query_map(params![target_id], Self::map_call_entry)?;
            let mut e = Vec::new(); for r in rows { e.push(r?); } Ok(e)
        } else {
            let mut stmt = conn.prepare("WITH RECURSIVE tc AS (SELECT r.source_id,r.target_id,1 AS depth FROM relationships r WHERE r.target_id=?1 AND r.rel_kind='calls' UNION ALL SELECT r.source_id,r.target_id,tc.depth+1 FROM relationships r JOIN tc ON r.target_id=tc.source_id WHERE tc.depth<?2 AND r.rel_kind='calls') SELECT DISTINCT s.id,s.name,s.kind,f.path,s.line,tc.depth FROM tc JOIN symbols s ON s.id=tc.source_id JOIN files f ON s.file_id=f.id ORDER BY tc.depth,s.name")?;
            let rows = stmt.query_map(params![target_id, depth], Self::map_call_entry)?;
            let mut e = Vec::new(); for r in rows { e.push(r?); } Ok(e)
        }
    }

    pub fn get_callees(&self, source_id: i64, depth: u32) -> anyhow::Result<Vec<SymbolCallEntry>> {
        let conn = self.conn.lock().unwrap();
        if depth <= 1 {
            let mut stmt = conn.prepare("SELECT s.id,s.name,s.kind,f.path,s.line,1 AS depth FROM relationships r JOIN symbols s ON s.id=r.target_id JOIN files f ON s.file_id=f.id WHERE r.source_id=?1 AND r.rel_kind='calls' ORDER BY s.name")?;
            let rows = stmt.query_map(params![source_id], Self::map_call_entry)?;
            let mut e = Vec::new(); for r in rows { e.push(r?); } Ok(e)
        } else {
            let mut stmt = conn.prepare("WITH RECURSIVE tc AS (SELECT r.source_id,r.target_id,1 AS depth FROM relationships r WHERE r.source_id=?1 AND r.rel_kind='calls' UNION ALL SELECT r.source_id,r.target_id,tc.depth+1 FROM relationships r JOIN tc ON r.source_id=tc.target_id WHERE tc.depth<?2 AND r.rel_kind='calls') SELECT DISTINCT s.id,s.name,s.kind,f.path,s.line,tc.depth FROM tc JOIN symbols s ON s.id=tc.target_id JOIN files f ON s.file_id=f.id ORDER BY tc.depth,s.name")?;
            let rows = stmt.query_map(params![source_id, depth], Self::map_call_entry)?;
            let mut e = Vec::new(); for r in rows { e.push(r?); } Ok(e)
        }
    }

    fn map_call_entry(row: &rusqlite::Row) -> rusqlite::Result<SymbolCallEntry> {
        Ok(SymbolCallEntry {
            id: row.get(0)?, name: row.get(1)?,
            kind: SymbolKind::from_node_type(&row.get::<_,String>(2)?).unwrap_or(SymbolKind::Function),
            file_path: row.get(3)?, line: row.get::<_,i64>(4)? as usize,
            depth: row.get::<_,i64>(5)? as u32,
        })
    }

    // ── Transactions ──

    pub fn begin_transaction(&self) -> anyhow::Result<()> { self.conn.lock().unwrap().execute_batch("BEGIN")?; Ok(()) }
    pub fn commit(&self) -> anyhow::Result<()> { self.conn.lock().unwrap().execute_batch("COMMIT")?; Ok(()) }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SymbolCallEntry {
    pub id: i64, pub name: String, pub kind: SymbolKind,
    pub file_path: String, pub line: usize, pub depth: u32,
}

// Tests use the main repo's test module pattern
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

    #[test] fn test_schema_creation() { let (s,_) = create_test_store(); assert!(!s.has_index().unwrap()); }
    #[test] fn test_upsert_file() { let (s,_) = create_test_store(); let fid = s.upsert_file("src/lib.rs","abc123").unwrap(); assert!(fid>0); assert_eq!(s.get_file_hash("src/lib.rs").unwrap(), Some("abc123".into())); }
    #[test] fn test_symbol_crud() { let (s,_) = create_test_store(); let fid = s.upsert_file("src/main.rs","abc").unwrap(); let sym = Symbol{id:None,name:"main".into(),kind:SymbolKind::Function,file_path:"src/main.rs".into(),line:1,col:1,signature:Some("fn main()".into()),visibility:Visibility::Private,parent_module:None}; let sid = s.insert_symbol(&sym,fid).unwrap(); assert!(sid>0); assert_eq!(s.get_symbol_by_id(sid).unwrap().unwrap().name,"main"); }
    #[test] fn test_file_deletion() { let (s,_) = create_test_store(); let fid = s.upsert_file("src/tmp.rs","def").unwrap(); s.insert_symbol(&Symbol{id:None,name:"tmp".into(),kind:SymbolKind::Function,file_path:"src/tmp.rs".into(),line:1,col:1,signature:None,visibility:Visibility::Private,parent_module:None},fid).unwrap(); s.delete_file("src/tmp.rs").unwrap(); assert!(s.get_symbols_for_file(fid).unwrap().is_empty()); }
    #[test] fn test_file_hash() { let dir = TempDir::new().unwrap(); let p = dir.path().join("t.txt"); fs::write(&p,b"hello").unwrap(); let h = IndexStore::file_hash(&p).unwrap(); assert_eq!(h.len(),64); assert_eq!(h, IndexStore::file_hash(&p).unwrap()); }
}
