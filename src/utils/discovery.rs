//! Daemon discovery file (`~/.wgenty-code/daemon.json`): lets UI processes
//! reuse an already-running global daemon instead of spawning a duplicate.
//! Writes are atomic (temp file + rename). The token ALSO stays in
//! `daemon.token` for existing readers (design §6.1).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const HEARTBEAT_INTERVAL_SECS: u64 = 30;
pub const HEARTBEAT_STALE_SECS: u64 = 120;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveryFile {
    pub port: u16,
    pub token: String,
    pub pid: u32,
    pub started_at: DateTime<Utc>,
    pub heartbeat_at: DateTime<Utc>,
}

pub fn discovery_file_path() -> PathBuf {
    crate::utils::config_dir().join("daemon.json")
}

pub fn write_discovery_file(file: &DiscoveryFile) -> anyhow::Result<()> {
    write_discovery_file_to(&discovery_file_path(), file)
}

pub fn read_discovery_file() -> Option<DiscoveryFile> {
    read_discovery_file_from(&discovery_file_path())
}

pub fn remove_discovery_file() -> anyhow::Result<()> {
    let path = discovery_file_path();
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    Ok(())
}

/// Remove the discovery file only if it still belongs to `pid`. Ownership
/// check mirrors `remove_daemon_token_if_matches`: with overlapping daemon
/// lifetimes (e.g. an idle-shutdown racing a fresh launch), an unconditional
/// delete removes the LIVE instance's discovery file.
pub fn remove_discovery_file_if_pid(pid: u32) -> anyhow::Result<()> {
    if read_discovery_file().map(|f| f.pid) == Some(pid) {
        remove_discovery_file()?;
    }
    Ok(())
}

/// A running daemon located via the discovery file.
#[derive(Debug, Clone)]
pub struct DiscoveredDaemon {
    pub port: u16,
    pub token: String,
}

/// Discovery decision chain (design §6.2): file exists and parses (else None)
/// → token matches `daemon.token` (else None) → heartbeat fresh (else None).
/// pid liveness is advisory only (cross-platform variance); heartbeat is
/// authoritative. Any failure = stale → caller falls back to spawning.
pub(crate) fn evaluate(
    file: Option<&DiscoveryFile>,
    expected_token: Option<&str>,
    now: DateTime<Utc>,
) -> Option<DiscoveredDaemon> {
    let file = file?;
    let expected = expected_token?;
    if file.token != expected {
        return None; // token mismatch → another daemon instance, do not connect
    }
    let age = now.signed_duration_since(file.heartbeat_at);
    if age.num_seconds() > HEARTBEAT_STALE_SECS as i64 {
        return None; // stale heartbeat → daemon likely dead
    }
    Some(DiscoveredDaemon {
        port: file.port,
        token: file.token.clone(),
    })
}

/// Try to locate an already-running daemon via the discovery file.
/// Returns None (caller falls back to spawning) when the file is missing,
/// corrupt, token-mismatched, its heartbeat is stale, or the port is not
/// accepting connections (daemon crashed but discovery file remains).
pub fn discover_daemon() -> Option<DiscoveredDaemon> {
    let file = read_discovery_file();
    let token = crate::utils::read_daemon_token();
    let found = evaluate(file.as_ref(), token.as_deref(), Utc::now())?;
    // TCP probe: verify the port is actually listening. A stale discovery
    // file (daemon crashed / was killed without clean shutdown) can have a
    // fresh-looking heartbeat if the process died between heartbeat writes.
    // The probe is a quick 200ms connect attempt; if it fails, treat the
    // daemon as dead so the caller falls back to spawning immediately
    // instead of waiting for the 120s heartbeat-stale window.
    if !probe_port(found.port) {
        tracing::debug!(
            port = found.port,
            "discovery file found but TCP probe failed; treating daemon as dead"
        );
        return None;
    }
    Some(found)
}

/// Quick TCP connect probe to `127.0.0.1:port`. Returns true if the port
/// accepts a connection within 200ms. Used by [`discover_daemon`] to verify
/// a discovered daemon is actually listening, not just that its heartbeat
/// file is fresh.
fn probe_port(port: u16) -> bool {
    use std::net::TcpStream;
    use std::time::Duration;
    let addr = format!("127.0.0.1:{}", port);
    TcpStream::connect_timeout(
        &addr
            .parse()
            .unwrap_or_else(|_| "127.0.0.1:0".parse().unwrap()),
        Duration::from_millis(200),
    )
    .is_ok()
}

/// Write the discovery file and spawn the heartbeat task that refreshes
/// `heartbeat_at` every [`HEARTBEAT_INTERVAL_SECS`]. Write failures are
/// non-fatal: discovery is additive and the token file path still works.
/// Returns the heartbeat task handle (detached when dropped).
pub fn spawn_discovery_writer(port: u16, token: String) -> tokio::task::JoinHandle<()> {
    let discovery = DiscoveryFile {
        port,
        token,
        pid: std::process::id(),
        started_at: Utc::now(),
        heartbeat_at: Utc::now(),
    };
    if let Err(e) = write_discovery_file(&discovery) {
        tracing::warn!(error = %e, "failed to write daemon discovery file");
    }
    tokio::spawn(async move {
        let mut ticker =
            tokio::time::interval(std::time::Duration::from_secs(HEARTBEAT_INTERVAL_SECS));
        loop {
            ticker.tick().await;
            let mut f = discovery.clone();
            f.heartbeat_at = Utc::now();
            if let Err(e) = write_discovery_file(&f) {
                tracing::warn!(error = %e, "daemon discovery heartbeat write failed");
            }
        }
    })
}

fn write_discovery_file_to(path: &Path, file: &DiscoveryFile) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    let body = serde_json::to_string(file)?;
    std::fs::write(&tmp, body)?;
    // Restrict permissions BEFORE the rename: the file carries the API token,
    // and chmod-after-rename would leave a window where the world-readable
    // default (umask) file is visible at its final path.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(&tmp, path)?; // atomic on all supported platforms
    Ok(())
}

fn read_discovery_file_from(path: &Path) -> Option<DiscoveryFile> {
    let body = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&body).ok() // corrupt → None (treated as absent)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn discovery_file_roundtrip_and_corruption_tolerance() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("daemon.json");
        let file = DiscoveryFile {
            port: 8371,
            token: "tok".into(),
            pid: 123,
            started_at: Utc::now(),
            heartbeat_at: Utc::now(),
        };
        write_discovery_file_to(&path, &file).expect("write");
        let back = read_discovery_file_from(&path).expect("reads back");
        assert_eq!(back.port, 8371);
        assert_eq!(back.token, "tok");

        std::fs::write(&path, b"{ not json").expect("corrupt");
        assert!(read_discovery_file_from(&path).is_none()); // 损坏 → None，不 panic
    }

    #[test]
    fn evaluate_matrix() {
        let now = Utc::now();
        let fresh = DiscoveryFile {
            port: 8371,
            token: "t".into(),
            pid: 1,
            started_at: now,
            heartbeat_at: now,
        };
        assert_eq!(
            evaluate(Some(&fresh), Some("t"), now).map(|d| d.port),
            Some(8371)
        );
        assert!(evaluate(Some(&fresh), Some("other"), now).is_none()); // token 不匹配
        assert!(evaluate(Some(&fresh), None, now).is_none()); // 无本地 token
        assert!(evaluate(None, Some("t"), now).is_none()); // 无文件
        let mut stale = fresh.clone();
        stale.heartbeat_at = now - chrono::Duration::seconds(HEARTBEAT_STALE_SECS as i64 + 1);
        assert!(evaluate(Some(&stale), Some("t"), now).is_none()); // 心跳过期
    }

    #[cfg(unix)]
    #[test]
    fn discovery_file_is_owner_only_on_unix() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("daemon.json");
        let file = DiscoveryFile {
            port: 8371,
            token: "tok".into(),
            pid: 123,
            started_at: Utc::now(),
            heartbeat_at: Utc::now(),
        };
        write_discovery_file_to(&path, &file).expect("write");
        let mode = std::fs::metadata(&path)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "discovery file contains the token");
    }
}
