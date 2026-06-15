use serde::Serialize;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::Mutex;

/// A single entry in the audit log.
#[derive(Debug, Serialize)]
pub struct AuditEntry {
    pub ts: String,
    pub audit_id: String,
    pub query_type: String,
    #[serde(skip_serializing_if = "serde_json::Value::is_null")]
    pub params: serde_json::Value,
    pub result_count: usize,
    pub elapsed_ms: u64,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub source_files: Vec<String>,
}

/// Append-only JSONL audit logger for codegraph queries.
pub struct AuditLogger {
    writer: Mutex<BufWriter<File>>,
}

impl AuditLogger {
    /// Create a new AuditLogger. Opens/creates the log file in append mode.
    pub fn new(log_path: &Path) -> std::io::Result<Self> {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_path)?;
        Ok(Self {
            writer: Mutex::new(BufWriter::new(file)),
        })
    }

    /// Write a single audit entry as a JSON line.
    pub fn log_query(&self, entry: &AuditEntry) -> std::io::Result<()> {
        let mut w = self.writer.lock().unwrap();
        let line = serde_json::to_string(entry)?;
        writeln!(w, "{}", line)?;
        w.flush()
    }

    /// Generate a unique audit ID (UUID v4).
    pub fn generate_audit_id() -> String {
        uuid::Uuid::new_v4().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audit_logger_new_creates_empty_file() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("audit.log");
        let logger = AuditLogger::new(&log_path).unwrap();
        drop(logger);
        assert!(log_path.exists());
        let content = std::fs::read_to_string(&log_path).unwrap();
        assert!(content.is_empty(), "audit.log should be empty after creation");
    }

    #[test]
    fn test_audit_logger_append_writes_json_line() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("audit.log");
        let logger = AuditLogger::new(&log_path).unwrap();

        let entry = AuditEntry {
            ts: "2026-06-15T10:30:00Z".to_string(),
            audit_id: "550e8400".to_string(),
            query_type: "codegraph_node".to_string(),
            params: serde_json::json!({"symbol": "ToolRegistry"}),
            result_count: 1,
            elapsed_ms: 12,
            source_files: vec!["src/tools/mod.rs".to_string()],
        };
        logger.log_query(&entry).unwrap();
        drop(logger);

        let content = std::fs::read_to_string(&log_path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(parsed["audit_id"], "550e8400");
        assert_eq!(parsed["query_type"], "codegraph_node");
    }

    #[test]
    fn test_generate_audit_id_is_unique() {
        let id1 = AuditLogger::generate_audit_id();
        let id2 = AuditLogger::generate_audit_id();
        assert_ne!(id1, id2);
        assert!(id1.len() > 8);
    }

    #[test]
    fn test_concurrent_writes_dont_corrupt() {
        use std::thread;
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("audit.log");
        let logger = std::sync::Arc::new(AuditLogger::new(&log_path).unwrap());
        let mut handles = vec![];
        for i in 0..5 {
            let log = logger.clone();
            handles.push(thread::spawn(move || {
                log.log_query(&AuditEntry {
                    ts: format!("2026-01-01T00:00:0{i}Z"),
                    audit_id: format!("id-{i}"),
                    query_type: "test".to_string(),
                    params: serde_json::json!({}),
                    result_count: 0,
                    elapsed_ms: 0,
                    source_files: vec![],
                })
                .unwrap();
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        drop(logger);

        let content = std::fs::read_to_string(&log_path).unwrap();
        let lines: Vec<_> = content.lines().filter(|l| !l.is_empty()).collect();
        assert_eq!(lines.len(), 5, "should have 5 lines, one per thread");
    }
}
