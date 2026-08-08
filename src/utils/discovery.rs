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
