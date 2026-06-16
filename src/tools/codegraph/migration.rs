//! Schema migration for the codegraph index database.
//!
//! Tracks schema version in a `meta` table and applies migrations
//! sequentially to bring the database up to date.

use rusqlite::Connection;

/// Current schema version (increment when changing the schema).
const CURRENT_VERSION: u32 = 2;

/// Handles versioned schema migrations for the codegraph index database.
pub struct SchemaMigration {
    conn: Connection,
}

impl SchemaMigration {
    pub fn new(conn: Connection) -> Self {
        Self { conn }
    }

    /// Detect the current schema version.
    /// Returns 0 if the meta table doesn't exist (fresh database).
    pub fn detect_version(&self) -> anyhow::Result<u32> {
        // Check if meta table exists
        let table_exists: bool = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='meta'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or(0)
            > 0;

        if !table_exists {
            return Ok(0);
        }

        // Check if schema_version key exists
        let key_exists: bool = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM meta WHERE key='schema_version'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or(0)
            > 0;

        if !key_exists {
            return Ok(0);
        }

        let version: String = self.conn.query_row(
            "SELECT value FROM meta WHERE key='schema_version'",
            [],
            |row| row.get(0),
        )?;
        Ok(version.parse().unwrap_or(0))
    }

    /// Run all pending migrations to bring the database to CURRENT_VERSION.
    pub fn run(&mut self) -> anyhow::Result<u32> {
        let version = self.detect_version()?;

        if version >= CURRENT_VERSION {
            return Ok(version);
        }

        // Ensure meta table exists
        self.conn
            .execute_batch("CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT);")?;

        // Apply migrations sequentially
        let mut current = version;

        if current < 1 {
            current = self.migrate_to_v1()?;
        }
        if current < 2 {
            current = self.migrate_to_v2()?;
        }

        // Update the version
        self.conn.execute(
            "INSERT OR REPLACE INTO meta (key, value) VALUES ('schema_version', ?1)",
            rusqlite::params![current.to_string()],
        )?;

        Ok(current)
    }

    /// v0 → v1: Initial schema creation (only if tables don't exist).
    fn migrate_to_v1(&mut self) -> anyhow::Result<u32> {
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
            CREATE INDEX IF NOT EXISTS idx_rels_target ON relationships(target_id);",
        )?;
        Ok(1)
    }

    /// v1 → v2: Add language column to symbols table.
    fn migrate_to_v2(&mut self) -> anyhow::Result<u32> {
        // Check if language column already exists
        let has_lang: bool = self
            .conn
            .prepare("SELECT language FROM symbols LIMIT 0")
            .is_ok();

        if !has_lang {
            self.conn.execute_batch(
                "ALTER TABLE symbols ADD COLUMN language TEXT NOT NULL DEFAULT 'rust';",
            )?;
        }
        Ok(2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_detect_version_on_fresh_db() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("test.db");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        let migration = SchemaMigration::new(conn);
        let version = migration.detect_version().unwrap();
        assert_eq!(version, 0);
    }

    #[test]
    fn test_run_migration_creates_schema_with_language() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("test.db");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        let mut migration = SchemaMigration::new(conn);
        migration.run().unwrap();

        let version = migration.detect_version().unwrap();
        assert_eq!(version, 2);
    }

    #[test]
    fn test_migration_is_idempotent() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("test.db");
        // First run
        {
            let conn = rusqlite::Connection::open(&db_path).unwrap();
            let mut migration = SchemaMigration::new(conn);
            migration.run().unwrap();
        }
        // Second run should not error
        {
            let conn = rusqlite::Connection::open(&db_path).unwrap();
            let mut migration = SchemaMigration::new(conn);
            migration.run().unwrap();
            assert_eq!(migration.detect_version().unwrap(), 2);
        }
    }

    #[test]
    fn test_migration_from_v1_to_v2_preserves_data() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("test.db");

        // Create a v1 schema (no language column)
        {
            let conn = rusqlite::Connection::open(&db_path).unwrap();
            conn.execute_batch(
                "CREATE TABLE files (
                    id INTEGER PRIMARY KEY,
                    path TEXT NOT NULL UNIQUE,
                    sha256 TEXT NOT NULL,
                    indexed_at TEXT NOT NULL DEFAULT (datetime('now'))
                );
                CREATE TABLE symbols (
                    id INTEGER PRIMARY KEY,
                    name TEXT NOT NULL,
                    kind TEXT NOT NULL,
                    file_id INTEGER NOT NULL REFERENCES files(id),
                    line INTEGER NOT NULL,
                    col INTEGER NOT NULL,
                    signature TEXT,
                    visibility TEXT,
                    parent_module TEXT
                );
                CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT);
                INSERT INTO meta VALUES ('schema_version', '1');
                INSERT INTO files VALUES (1, 'test.rs', 'abc', datetime('now'));
                INSERT INTO symbols VALUES (1, 'my_fn', 'function', 1, 10, 1, 'fn my_fn()', 'pub', NULL);",
            )
            .unwrap();
        }

        // Run migration
        {
            let conn = rusqlite::Connection::open(&db_path).unwrap();
            let mut migration = SchemaMigration::new(conn);
            migration.run().unwrap();
        }

        // Verify data preserved and language column exists
        {
            let conn = rusqlite::Connection::open(&db_path).unwrap();
            let lang: String = conn
                .query_row("SELECT language FROM symbols WHERE name='my_fn'", [], |r| {
                    r.get(0)
                })
                .unwrap();
            assert_eq!(lang, "rust");
        }
    }
}
