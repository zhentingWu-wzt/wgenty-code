# Database Guidelines

> ORM patterns, queries, and migrations in this project.

---

## Overview

Persistence is **SQLite via `rusqlite`, no ORM**. SQL is handwritten; stores own their schema and run migrations at open time. Canonical example: `src/transcript/store.rs` (`SubagentTranscriptStore`). `src/context/` follows the same pattern for session/context storage.

Database files live under user/app data dirs (e.g. `~/.wgenty-code/...`), never in the project tree.

---

## Store Pattern

```rust
// src/transcript/store.rs
pub struct SubagentTranscriptStore {
    db: Mutex<Connection>,   // rusqlite Connection is Send but not Sync
}

impl SubagentTranscriptStore {
    /// Open or create the database. Auto-creates tables and indexes.
    pub fn open(path: &Path) -> Result<Self, TranscriptError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(...)?;
        }
        let db = Connection::open(path)?;
        let store = Self { db: Mutex::new(db) };
        store.run_migrations()?;      // always, on every open
        Ok(store)
    }
}
```

Rules:
- Wrap `Connection` in `Mutex` (it's `Send` but not `Sync`); hold the lock per operation, never across `.await`.
- Typed store errors (e.g. `TranscriptError`) — no bare `rusqlite::Error` escaping the store layer.
- Use `params!` for values; **never** interpolate user data into SQL with `format!`. (The only `format!` in store.rs builds DDL from compile-time constants — that pattern is acceptable only for schema, not data.)

---

## Schema Conventions

```sql
PRAGMA journal_mode=WAL;      -- always, first thing
PRAGMA foreign_keys=ON;       -- always

CREATE TABLE IF NOT EXISTS subagent_transcripts (
    id TEXT PRIMARY KEY,
    ...
    created_at INTEGER DEFAULT (unixepoch('now'))
);
CREATE INDEX IF NOT EXISTS idx_transcripts_session ON subagent_transcripts(session_id, started_at DESC);
```

- Tables/indexes: `CREATE ... IF NOT EXISTS` so re-opens are safe.
- Foreign keys declared with `REFERENCES ... ON DELETE CASCADE` where child rows shouldn't outlive parents (`subagent_events` → `subagent_transcripts`).
- Every read query path has a matching index; add the index in the same migration as the table.
- Timestamps: `created_at` uses unixepoch **seconds** (SQLite default); `started_at`/`finished_at` use **unix milliseconds** — units are documented in a schema comment because the mix is deliberate. Keep the comment when adding time columns.

---

## Migrations

Migrations are **idempotent and run on every open** (`run_migrations()`). SQLite's `ALTER TABLE ADD COLUMN` has no `IF NOT EXISTS`, so guard each column addition with a `PRAGMA table_info` presence check:

```rust
fn column_exists(db: &Connection, table: &str, column: &str) -> Result<bool, rusqlite::Error> {
    let mut stmt = db.prepare(&format!("PRAGMA table_info({table})"))?;
    // ...
}

for (col, decl) in [("root_cause", "TEXT"), /* ... */] {
    if !column_exists(&db, "subagent_transcripts", col)? {
        db.execute_batch(&format!("ALTER TABLE subagent_transcripts ADD COLUMN {col} {decl}"))?;
    }
}
```

This keeps re-opens safe and upgrades legacy databases without a version table. Do not add a schema-version table unless a future migration can't be expressed this way.

---

## Testing Requirements

- **Migration idempotency**: re-running migrations on an already-migrated DB must not error (`src/transcript/store.rs` has this test).
- **Legacy-schema compat**: test opening a database created with the *pre-migration* schema — it must migrate cleanly.
- Use in-file `#[cfg(test)] mod tests` with tempdir-backed DB files.

---

## Common Mistakes

- Forgetting `PRAGMA foreign_keys=ON` — SQLite ships with FK enforcement **off** by default.
- Holding the `Mutex<Connection>` guard across an `.await` — blocks the executor and risks deadlock.
- Adding a column via bare `ALTER TABLE` without the `column_exists` guard — crashes on second open.
- Mixing timestamp units without a schema comment (seconds vs milliseconds).
