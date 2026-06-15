//! SQLite-backed SubagentTranscriptStore.

use super::{SubagentEventRecord, SubagentTranscript, SubagentTranscriptHeader, TranscriptError, TranscriptStatus};
use rusqlite::{params, Connection};
use std::path::Path;

pub struct SubagentTranscriptStore {
    db: Connection,
}

impl SubagentTranscriptStore {
    /// Open or create the database. Auto-creates tables and indexes.
    pub fn open(path: &Path) -> Result<Self, TranscriptError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| TranscriptError::Io(e.to_string()))?;
        }
        let db = Connection::open(path)?;
        let store = Self { db };
        store.run_migrations()?;
        Ok(store)
    }

    fn run_migrations(&self) -> Result<(), TranscriptError> {
        self.db.execute_batch("
            PRAGMA journal_mode=WAL;
            PRAGMA foreign_keys=ON;

            CREATE TABLE IF NOT EXISTS subagent_transcripts (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                parent_id TEXT,
                label TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'pending',
                system_prompt TEXT,
                user_prompt TEXT NOT NULL,
                started_at INTEGER NOT NULL,
                finished_at INTEGER,
                total_tokens INTEGER DEFAULT 0,
                max_rounds INTEGER,
                actual_rounds INTEGER DEFAULT 0,
                token_budget_k INTEGER,
                error_message TEXT,
                summary TEXT,
                -- created_at uses unixepoch seconds; started_at/finished_at use unix ms
                created_at INTEGER DEFAULT (unixepoch('now'))
            );

            CREATE TABLE IF NOT EXISTS subagent_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                transcript_id TEXT NOT NULL REFERENCES subagent_transcripts(id) ON DELETE CASCADE,
                round INTEGER NOT NULL,
                event_type TEXT NOT NULL,
                tool_name TEXT,
                tool_params TEXT,
                data TEXT NOT NULL,
                elapsed_ms INTEGER NOT NULL,
                token_count INTEGER,
                created_at INTEGER DEFAULT (unixepoch('now'))
            );

            CREATE INDEX IF NOT EXISTS idx_transcripts_session ON subagent_transcripts(session_id, started_at DESC);
            CREATE INDEX IF NOT EXISTS idx_transcripts_status ON subagent_transcripts(status);
            CREATE INDEX IF NOT EXISTS idx_events_transcript ON subagent_events(transcript_id, round);
            CREATE INDEX IF NOT EXISTS idx_events_type ON subagent_events(event_type);
        ")?;
        Ok(())
    }

    /// Save a full transcript (header + all events) in a single transaction.
    ///
    /// If `retention_days` is `Some(d)` and `d > 0`, also deletes transcripts
    /// older than `d` days within the same transaction.
    pub fn save(&self, transcript: &SubagentTranscript, retention_days: Option<u32>) -> Result<(), TranscriptError> {
        let tx = self.db.unchecked_transaction()?;

        let status_str = match transcript.status {
            TranscriptStatus::Pending => "pending",
            TranscriptStatus::Running => "running",
            TranscriptStatus::Completed => "completed",
            TranscriptStatus::Failed => "failed",
            TranscriptStatus::Cancelled => "cancelled",
        };

        tx.execute(
            "INSERT OR REPLACE INTO subagent_transcripts
             (id, session_id, parent_id, label, status, system_prompt, user_prompt,
              started_at, finished_at, total_tokens, max_rounds, actual_rounds,
              token_budget_k, error_message, summary)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            params![
                transcript.id,
                transcript.session_id,
                transcript.parent_id,
                transcript.label,
                status_str,
                transcript.system_prompt,
                transcript.user_prompt,
                transcript.started_at,
                transcript.finished_at,
                transcript.total_tokens,
                transcript.max_rounds,
                transcript.actual_rounds,
                transcript.token_budget_k,
                transcript.error_message,
                transcript.summary,
            ],
        )?;

        // Insert events
        for event in &transcript.events {
            tx.execute(
                "INSERT INTO subagent_events (transcript_id, round, event_type, tool_name, tool_params, data, elapsed_ms, token_count)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    transcript.id,
                    event.round,
                    event.event_type,
                    event.tool_name,
                    event.tool_params.as_ref().map(|v| v.to_string()),
                    event.data,
                    event.elapsed_ms,
                    event.token_count,
                ],
            )?;
        }

        // Clean up old transcripts in the same transaction if retention_days is set
        if let Some(days) = retention_days {
            if days > 0 {
                tx.execute(
                    "DELETE FROM subagent_transcripts WHERE started_at < (unixepoch('now') - ?1 * 86400) * 1000",
                    params![days],
                )?;
            }
        }

        tx.commit()?;
        Ok(())
    }

    /// Update transcript header status (for checkpoint).
    pub fn checkpoint_status(&self, id: &str, status: TranscriptStatus, round: u32, tokens: u64) -> Result<(), TranscriptError> {
        let status_str = status.to_string();
        let rows_affected = self.db.execute(
            "UPDATE subagent_transcripts SET status = ?1, actual_rounds = ?2, total_tokens = ?3 WHERE id = ?4",
            params![status_str, round, tokens, id],
        )?;
        if rows_affected == 0 {
            return Err(TranscriptError::NotFound(id.to_string()));
        }
        Ok(())
    }

    /// Append events to an existing transcript.
    pub fn append_events(&self, transcript_id: &str, events: &[SubagentEventRecord]) -> Result<(), TranscriptError> {
        let tx = self.db.unchecked_transaction()?;
        for event in events {
            tx.execute(
                "INSERT INTO subagent_events (transcript_id, round, event_type, tool_name, tool_params, data, elapsed_ms, token_count)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    transcript_id,
                    event.round,
                    event.event_type,
                    event.tool_name,
                    event.tool_params.as_ref().map(|v| v.to_string()),
                    event.data,
                    event.elapsed_ms,
                    event.token_count,
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// List transcripts by session (headers only, no events).
    pub fn list_by_session(&self, session_id: &str) -> Result<Vec<SubagentTranscriptHeader>, TranscriptError> {
        let mut stmt = self.db.prepare(
            "SELECT id, session_id, parent_id, label, status, started_at, finished_at,
                    total_tokens, actual_rounds, error_message, summary
             FROM subagent_transcripts
             WHERE session_id = ?1
             ORDER BY started_at DESC"
        )?;
        let rows = stmt.query_map(params![session_id], |row| {
            Ok(SubagentTranscriptHeader {
                id: row.get(0)?,
                session_id: row.get(1)?,
                parent_id: row.get(2)?,
                label: row.get(3)?,
                status: row.get(4)?,
                started_at: row.get(5)?,
                finished_at: row.get(6)?,
                total_tokens: row.get::<_, i64>(7)? as u64,
                actual_rounds: row.get::<_, i32>(8)? as u32,
                error_message: row.get(9)?,
                summary: row.get(10)?,
            })
        })?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    /// Get full transcript by ID (with all events).
    pub fn get_by_id(&self, id: &str) -> Result<Option<SubagentTranscript>, TranscriptError> {
        let mut stmt = self.db.prepare(
            "SELECT id, session_id, parent_id, label, status, system_prompt, user_prompt,
                    started_at, finished_at, total_tokens, max_rounds, actual_rounds,
                    token_budget_k, error_message, summary
             FROM subagent_transcripts WHERE id = ?1"
        )?;
        let mut rows = stmt.query_map(params![id], |row| {
            Ok(SubagentTranscript {
                id: row.get(0)?,
                session_id: row.get(1)?,
                parent_id: row.get(2)?,
                label: row.get(3)?,
                status: match row.get::<_, String>(4)?.as_str() {
                    "completed" => TranscriptStatus::Completed,
                    "failed" => TranscriptStatus::Failed,
                    "cancelled" => TranscriptStatus::Cancelled,
                    "running" => TranscriptStatus::Running,
                    _ => TranscriptStatus::Pending,
                },
                system_prompt: row.get(5)?,
                user_prompt: row.get(6)?,
                started_at: row.get(7)?,
                finished_at: row.get(8)?,
                total_tokens: row.get::<_, i64>(9)? as u64,
                max_rounds: row.get::<_, Option<i32>>(10)?.map(|v| v as u32),
                actual_rounds: row.get::<_, i32>(11)? as u32,
                token_budget_k: row.get::<_, Option<i64>>(12)?.map(|v| v as u64),
                error_message: row.get(13)?,
                summary: row.get(14)?,
                events: Vec::new(),
            })
        })?;

        if let Some(transcript_result) = rows.next() {
            let mut transcript = transcript_result?;
            // Load events
            let mut evt_stmt = self.db.prepare(
                "SELECT round, event_type, tool_name, tool_params, data, elapsed_ms, token_count
                 FROM subagent_events WHERE transcript_id = ?1 ORDER BY round, id"
            )?;
            let evt_rows = evt_stmt.query_map(params![id], |row| {
                Ok(SubagentEventRecord {
                    round: row.get::<_, i32>(0)? as u32,
                    event_type: row.get(1)?,
                    tool_name: row.get(2)?,
                    tool_params: row.get::<_, Option<String>>(3)?.and_then(|s| {
                        match serde_json::from_str(&s) {
                            Ok(v) => Some(v),
                            Err(e) => {
                                tracing::warn!(tool_params = %s, error = %e, "failed to deserialize tool_params");
                                None
                            }
                        }
                    }),
                    data: row.get(4)?,
                    elapsed_ms: row.get::<_, i64>(5)? as u64,
                    token_count: row.get::<_, Option<i64>>(6)?.map(|v| v as u64),
                })
            })?;
            for evt in evt_rows {
                transcript.events.push(evt?);
            }
            Ok(Some(transcript))
        } else {
            Ok(None)
        }
    }

    /// Search transcripts by label (fuzzy match).
    pub fn search(&self, query: &str) -> Result<Vec<SubagentTranscriptHeader>, TranscriptError> {
        if query.trim().is_empty() {
            return Ok(Vec::new());
        }
        let pattern = format!("%{}%", query);
        let mut stmt = self.db.prepare(
            "SELECT id, session_id, parent_id, label, status, started_at, finished_at,
                    total_tokens, actual_rounds, error_message, summary
             FROM subagent_transcripts
             WHERE label LIKE ?1
             ORDER BY started_at DESC
             LIMIT 100"
        )?;
        let rows = stmt.query_map(params![pattern], |row| {
            Ok(SubagentTranscriptHeader {
                id: row.get(0)?,
                session_id: row.get(1)?,
                parent_id: row.get(2)?,
                label: row.get(3)?,
                status: row.get(4)?,
                started_at: row.get(5)?,
                finished_at: row.get(6)?,
                total_tokens: row.get::<_, i64>(7)? as u64,
                actual_rounds: row.get::<_, i32>(8)? as u32,
                error_message: row.get(9)?,
                summary: row.get(10)?,
            })
        })?;
        let mut result = Vec::new();
        for row in rows {
            result.push(row?);
        }
        Ok(result)
    }

    /// Delete transcripts older than retention_days.
    pub fn cleanup(&self, retention_days: u32) -> Result<usize, TranscriptError> {
        if retention_days == 0 {
            return Ok(0);
        }
        let deleted = self.db.execute(
            "DELETE FROM subagent_transcripts WHERE started_at < (unixepoch('now') - ?1 * 86400) * 1000",
            params![retention_days],
        )?;
        Ok(deleted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup_store() -> (SubagentTranscriptStore, TempDir) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.db");
        let store = SubagentTranscriptStore::open(&path).unwrap();
        (store, dir)
    }

    fn sample_transcript(id: &str, session_id: &str) -> SubagentTranscript {
        SubagentTranscript {
            id: id.to_string(),
            session_id: session_id.to_string(),
            parent_id: None,
            label: format!("test-{}", id),
            status: TranscriptStatus::Completed,
            system_prompt: None,
            user_prompt: "do something".to_string(),
            started_at: 1000,
            finished_at: Some(2000),
            total_tokens: 500,
            max_rounds: Some(10),
            actual_rounds: 3,
            token_budget_k: None,
            error_message: None,
            summary: Some("done".to_string()),
            events: vec![
                SubagentEventRecord {
                    round: 0,
                    event_type: "thought".to_string(),
                    tool_name: None,
                    tool_params: None,
                    data: "analyzing...".to_string(),
                    elapsed_ms: 100,
                    token_count: Some(50),
                },
                SubagentEventRecord {
                    round: 1,
                    event_type: "action".to_string(),
                    tool_name: Some("file_read".to_string()),
                    tool_params: Some(serde_json::json!({"path": "src/main.rs"})),
                    data: "reading file".to_string(),
                    elapsed_ms: 500,
                    token_count: Some(200),
                },
            ],
        }
    }

    #[test]
    fn test_save_and_get_by_id() {
        let (store, _dir) = setup_store();
        let t = sample_transcript("test-1", "session-1");
        store.save(&t, None).unwrap();

        let loaded = store.get_by_id("test-1").unwrap().unwrap();
        assert_eq!(loaded.id, "test-1");
        assert_eq!(loaded.session_id, "session-1");
        assert_eq!(loaded.events.len(), 2);
        assert_eq!(loaded.events[0].event_type, "thought");
    }

    #[test]
    fn test_list_by_session() {
        let (store, _dir) = setup_store();
        store.save(&sample_transcript("a", "sess-1"), None).unwrap();
        store.save(&sample_transcript("b", "sess-1"), None).unwrap();
        store.save(&sample_transcript("c", "sess-2"), None).unwrap();

        let list = store.list_by_session("sess-1").unwrap();
        assert_eq!(list.len(), 2);

        let list2 = store.list_by_session("sess-2").unwrap();
        assert_eq!(list2.len(), 1);
    }

    #[test]
    fn test_search() {
        let (store, _dir) = setup_store();
        store.save(&sample_transcript("a", "s1"), None).unwrap();
        let mut t2 = sample_transcript("b", "s1");
        t2.label = "special-fix".to_string();
        store.save(&t2, None).unwrap();

        let results = store.search("special").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "b");
    }

    #[test]
    fn test_checkpoint_and_append() {
        let (store, _dir) = setup_store();
        let t = sample_transcript("cp-1", "session-1");
        store.save(&t, None).unwrap();

        store.checkpoint_status("cp-1", TranscriptStatus::Running, 5, 1000).unwrap();

        let loaded = store.get_by_id("cp-1").unwrap().unwrap();
        assert_eq!(loaded.actual_rounds, 5);
        assert_eq!(loaded.total_tokens, 1000);

        store.append_events("cp-1", &[
            SubagentEventRecord {
                round: 2,
                event_type: "completion".to_string(),
                tool_name: None,
                tool_params: None,
                data: "done".to_string(),
                elapsed_ms: 3000,
                token_count: None,
            },
        ]).unwrap();

        let loaded2 = store.get_by_id("cp-1").unwrap().unwrap();
        assert_eq!(loaded2.events.len(), 3);
    }

    #[test]
    fn test_cleanup() {
        let (store, _dir) = setup_store();
        let mut t = sample_transcript("old", "s1");
        t.started_at = 100; // very old timestamp (unix ms)
        store.save(&t, None).unwrap();

        let mut t2 = sample_transcript("new", "s1");
        // Set a recent timestamp (current unix ms) so it won't be deleted
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        t2.started_at = now_ms;
        store.save(&t2, None).unwrap();

        let deleted = store.cleanup(1).unwrap(); // 1 day retention
        assert_eq!(deleted, 1);

        assert!(store.get_by_id("old").unwrap().is_none());
        assert!(store.get_by_id("new").unwrap().is_some());
    }

    #[test]
    fn test_search_returns_up_to_100() {
        let (store, _dir) = setup_store();
        for i in 0..60 {
            let mut t = sample_transcript(&format!("batch-{}", i), "session-search-100");
            t.label = "batch-item".to_string();
            store.save(&t, None).unwrap();
        }
        let results = store.search("batch").unwrap();
        assert_eq!(results.len(), 60, "search should return all 60 results, not limited to 50");
    }

    #[test]
    fn test_search_empty_query_returns_empty() {
        let (store, _dir) = setup_store();
        store.save(&sample_transcript("a", "s1"), None).unwrap();
        store.save(&sample_transcript("b", "s1"), None).unwrap();

        let results = store.search("").unwrap();
        assert_eq!(results.len(), 0, "empty query should return no results");
    }

    #[test]
    fn test_token_budget_k_round_trip() {
        let (store, _dir) = setup_store();
        let mut t = sample_transcript("budget-test", "s1");
        t.token_budget_k = Some(42);
        store.save(&t, None).unwrap();

        let loaded = store.get_by_id("budget-test").unwrap().unwrap();
        assert_eq!(loaded.token_budget_k, Some(42));
    }

    #[test]
    fn test_checkpoint_status_not_found() {
        let (store, _dir) = setup_store();
        let result = store.checkpoint_status("non-existent", TranscriptStatus::Running, 1, 100);
        assert!(matches!(result, Err(TranscriptError::NotFound(_))));
    }

    #[test]
    fn test_cleanup_zero_guard() {
        let (store, _dir) = setup_store();
        let mut t = sample_transcript("zero-guard-old", "s1");
        t.started_at = 100;
        store.save(&t, None).unwrap();

        let deleted = store.cleanup(0).unwrap();
        assert_eq!(deleted, 0, "cleanup(0) should not delete anything");
        assert!(store.get_by_id("zero-guard-old").unwrap().is_some(), "old transcript should still exist");
    }
}
