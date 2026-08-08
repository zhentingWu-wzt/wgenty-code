//! Daemon lifecycle management for the Tauri shell.
//!
//! Mirrors the discovery + spawn logic from `src/tui/util.rs` and
//! `src/utils/discovery.rs`, but **without depending on the main crate** —
//! the shell stays a standalone crate. Daemon is spawned as a **separate
//! process** (`wgenty-code daemon --port <p>`), not embedded in-process,
//! because:
//!
//! 1. The daemon is designed as a standalone server (own CLI subcommand,
//!    token file, discovery heartbeat, multi-UI reuse).
//! 2. A separate process survives the Tauri shell exit, so other UIs (TUI,
//!    browser) can keep using the same daemon instance.
//! 3. Keeps the shell's compile graph tiny (no axum/reqwest/rusqlite/ratatui).
//!
//! Decision chain (mirrors `discovery::evaluate`):
//!   discovery file readable → token matches daemon.token → heartbeat fresh
//!   → connect to existing. Else spawn new, poll /health until ready.

use std::path::PathBuf;
use std::time::Duration;

use serde::Deserialize;

/// Heartbeat freshness threshold (mirrors `discovery::HEARTBEAT_STALE_SECS`).
const HEARTBEAT_STALE_SECS: i64 = 120;

/// How long to wait for a spawned daemon to become healthy.
const HEALTH_POLL_TIMEOUT_SECS: u64 = 15;
/// Initial interval between health polls, doubling each retry (exponential
/// backoff caps the wakeups during a slow daemon boot).
const HEALTH_POLL_INITIAL_MS: u64 = 200;

/// Discovery file payload (subset of `discovery::DiscoveryFile` — we only
/// need port, token, heartbeat to make the connect-vs-spawn decision).
#[derive(Debug, Deserialize)]
struct DiscoveryFile {
    port: u16,
    token: String,
    #[allow(dead_code)]
    pid: u32,
    heartbeat_at: String, // ISO 8601 — parsed loosely via chrono
}

/// Result of ensuring the daemon is running.
#[derive(Debug)]
#[allow(dead_code)] // port/token are read by callers once the shell wires health checks through Rust
pub struct DaemonHandle {
    pub port: u16,
    /// The bearer token to send on daemon API requests.
    pub token: String,
}

/// Config dir: `~/.wgenty-code` (mirrors `utils::config_dir`).
fn config_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".wgenty-code"))
}

fn discovery_file_path() -> Option<PathBuf> {
    config_dir().map(|d| d.join("daemon.json"))
}

fn token_file_path() -> Option<PathBuf> {
    config_dir().map(|d| d.join("daemon.token"))
}

/// Read the daemon bearer token from disk (best-effort, trimmed).
pub fn read_token() -> Option<String> {
    let path = token_file_path()?;
    std::fs::read_to_string(&path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Try to discover an already-running, healthy daemon.
///
/// Returns `Some(handle)` only when ALL of:
/// - `daemon.json` exists and parses
/// - its `token` field equals `daemon.token` file contents (same instance)
/// - its `heartbeat_at` is within `HEARTBEAT_STALE_SECS` of now
fn discover_daemon() -> Option<DaemonHandle> {
    let path = discovery_file_path()?;
    let body = std::fs::read_to_string(&path).ok()?;
    let file: DiscoveryFile = serde_json::from_str(&body).ok()?;

    let token = read_token()?;
    if file.token != token {
        return None; // different daemon instance
    }

    // Parse heartbeat loosely. We don't need sub-second precision — just
    // "is it within the last 2 minutes?". Chrono RFC3339 parse covers the
    // format `discovery::spawn_discovery_writer` emits (`DateTime<Utc>` →
    // ISO 8601 with timezone).
    let heartbeat = chrono::DateTime::parse_from_rfc3339(&file.heartbeat_at).ok()?;
    let now = chrono::Utc::now();
    let age = now.signed_duration_since(heartbeat.with_timezone(&chrono::Utc));
    if age.num_seconds() > HEARTBEAT_STALE_SECS {
        return None; // stale — likely dead
    }

    Some(DaemonHandle {
        port: file.port,
        token,
    })
}

/// Locate the `wgenty-code` daemon binary.
///
/// In dev: `../../target/{debug,release}/wgenty-code` (relative to
/// `desktop/src-tauri/`). In a packaged app: the binary is bundled via Tauri
/// `bundle.externalBin` and lands **next to the shell executable** (macOS:
/// `Contents/MacOS/`, Windows/Linux: the same directory as the shell binary),
/// with the target-triple suffix stripped. We check that directory first, then
/// the resource dir as a fallback, then dev.
///
/// Prefers **release** (faster daemon, less CPU/memory) when available, falling
/// back to debug. Both builds correctly emit discovery files after the
/// bind-before-write-token fix in `src/daemon/mod.rs`.
///
/// A daemon binary name is either the exact "wgenty-code[.exe]" or a
/// target-triple-suffixed externalBin name "wgenty-code-<triple>[.exe]".
/// The triple always contains '-' (e.g. "aarch64-apple-darwin"), which
/// excludes the shell's own binary ("wgenty-code-desktop") and non-binary
/// resources like "wgenty-code.icns".
fn is_daemon_binary_name(name: &str) -> bool {
    if name == "wgenty-code" || name == "wgenty-code.exe" {
        return true;
    }
    name.strip_prefix("wgenty-code-").is_some_and(|suffix| {
        let s = suffix.strip_suffix(".exe").unwrap_or(suffix);
        s.contains('-') && !s.contains('.')
    })
}

/// Scan `dir` for a daemon binary: exact name first, then the
/// target-triple-suffixed externalBin convention.
fn find_daemon_in_dir(dir: &std::path::Path, exe_name: &str) -> Option<PathBuf> {
    let direct = dir.join(exe_name);
    if direct.exists() {
        return Some(direct);
    }
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let name_str = entry.file_name().to_string_lossy().to_string();
            if is_daemon_binary_name(&name_str) {
                return Some(entry.path());
            }
        }
    }
    None
}

fn locate_daemon_binary(resource_dir: Option<&std::path::Path>) -> Option<PathBuf> {
    // Windows binaries have a .exe suffix.
    let exe_name = if cfg!(windows) {
        "wgenty-code.exe"
    } else {
        "wgenty-code"
    };

    // 1. Packaged app: the externalBin daemon sits next to the shell executable
    //    (Tauri strips the target-triple suffix when bundling). Prefer this —
    //    it is where the binary actually lands on all three platforms.
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            if let Some(found) = find_daemon_in_dir(dir, exe_name) {
                return Some(found);
            }
        }
    }

    // 2. Packaged app (fallback): check the Tauri resource directory.
    //    externalBin binaries are suffixed with the target triple, e.g.
    //    "wgenty-code-aarch64-apple-darwin". We strip the suffix to find it.
    if let Some(dir) = resource_dir {
        if let Some(found) = find_daemon_in_dir(dir, exe_name) {
            return Some(found);
        }
    }

    // 3. Dev mode: check target/{release,debug}/ relative to repo root.
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir
        .parent() // desktop/
        .and_then(|p| p.parent())?; // repo root

    let release = repo_root.join("target/release").join(exe_name);
    if release.exists() {
        return Some(release);
    }
    let debug = repo_root.join("target/debug").join(exe_name);
    if debug.exists() {
        return Some(debug);
    }
    None
}

/// Spawn a new daemon process on the default port (8371).
///
/// Detaches the child — we intentionally do NOT track/kill it on shell exit,
/// because the daemon is designed for multi-UI reuse (other shells / TUI /
/// browser may be connected). Its lifetime is governed by its own discovery
/// heartbeat and external process management.
fn spawn_daemon(port: u16, resource_dir: Option<&std::path::Path>) -> Result<(), String> {
    let exe = locate_daemon_binary(resource_dir)
        .ok_or_else(|| "wgenty-code binary not found (expected at target/{debug,release}/wgenty-code or bundled resource)".to_string())?;

    // Detached spawn: the daemon writes its own token + discovery files,
    // which we'll pick up via read_token() / discover_daemon() afterwards.
    std::process::Command::new(&exe)
        .args(["daemon", "--port", &port.to_string()])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("failed to spawn daemon at {}: {}", exe.display(), e))?;

    Ok(())
}

/// Poll the daemon's `/health` endpoint until it responds or we time out.
///
/// Uses a raw TCP connect check (not an HTTP request with auth) because
/// `/health` is the one public endpoint that doesn't require a bearer token
/// (see `src/daemon/routes.rs`). This lets us confirm "daemon is up" before
/// the token file is necessarily written.
async fn wait_for_health(port: u16) -> Result<(), String> {
    let url = format!("http://127.0.0.1:{}/api/v1/health", port);
    let deadline = std::time::Instant::now() + Duration::from_secs(HEALTH_POLL_TIMEOUT_SECS);
    let mut delay_ms = HEALTH_POLL_INITIAL_MS;

    loop {
        // /health is public (no auth needed) — a simple reqwest-free check.
        // We avoid adding reqwest as a dep by using a bare TCP connect.
        if tokio::net::TcpStream::connect(format!("127.0.0.1:{}", port))
            .await
            .is_ok()
        {
            // Port is accepting connections — daemon is likely up. The token
            // file may still be racing, so the caller re-reads it after this
            // returns.
            let _ = url; // kept for clarity / future HTTP-based check
            return Ok(());
        }

        if std::time::Instant::now() >= deadline {
            return Err(format!(
                "daemon did not become reachable on port {} within {}s",
                port, HEALTH_POLL_TIMEOUT_SECS
            ));
        }

        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
        delay_ms = (delay_ms * 2).min(2000); // cap backoff at 2s
    }
}

/// Ensure the daemon is running and return a handle to it.
///
/// Strategy: discover existing → if healthy, reuse. Else spawn new → wait for
/// health → re-read token → return handle.
///
/// `resource_dir`: when running inside a packaged Tauri app, pass the resolved
/// resource directory so `locate_daemon_binary` can find the bundled daemon
/// binary. In dev mode, pass `None` (uses target/{debug,release}/).
pub async fn ensure_daemon(resource_dir: Option<PathBuf>) -> Result<DaemonHandle, String> {
    const DEFAULT_PORT: u16 = 8371;

    // 1. Try to discover an already-running, healthy daemon.
    if let Some(handle) = discover_daemon() {
        return Ok(handle);
    }

    // 2. No healthy daemon found — spawn one.
    spawn_daemon(DEFAULT_PORT, resource_dir.as_deref())?;

    // 3. Wait for it to accept connections.
    wait_for_health(DEFAULT_PORT).await?;

    // 4. The spawned daemon writes its token to daemon.token. Re-read it
    //    (with a short retry — the write may lag the port-bind by a moment).
    for _ in 0..20 {
        if let Some(token) = read_token() {
            return Ok(DaemonHandle {
                port: DEFAULT_PORT,
                token,
            });
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    Err("daemon spawned but token file never appeared".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daemon_binary_name_matches_exact() {
        assert!(is_daemon_binary_name("wgenty-code"));
        assert!(is_daemon_binary_name("wgenty-code.exe"));
    }

    #[test]
    fn daemon_binary_name_matches_triple_suffix() {
        // ExternalBin convention: wgenty-code-<target-triple>[.exe].
        assert!(is_daemon_binary_name("wgenty-code-aarch64-apple-darwin"));
        assert!(is_daemon_binary_name("wgenty-code-x86_64-apple-darwin"));
        assert!(is_daemon_binary_name(
            "wgenty-code-x86_64-unknown-linux-gnu"
        ));
        assert!(is_daemon_binary_name(
            "wgenty-code-x86_64-pc-windows-msvc.exe"
        ));
    }

    #[test]
    fn daemon_binary_name_rejects_shell_and_resources() {
        // The shell's own binary must never be picked as the daemon.
        assert!(!is_daemon_binary_name("wgenty-code-desktop"));
        assert!(!is_daemon_binary_name("wgenty-code-desktop.exe"));
        // Non-binary resources sharing the prefix.
        assert!(!is_daemon_binary_name("wgenty-code.icns"));
        assert!(!is_daemon_binary_name("wgenty-code-README.md"));
        assert!(!is_daemon_binary_name("wgenty-code-"));
    }

    #[test]
    fn find_in_dir_prefers_exact_name() {
        let tmp = tempfile::tempdir().unwrap();
        let exact = tmp.path().join("wgenty-code");
        let suffixed = tmp.path().join("wgenty-code-aarch64-apple-darwin");
        std::fs::write(&exact, b"daemon").unwrap();
        std::fs::write(&suffixed, b"daemon").unwrap();

        let found = find_daemon_in_dir(tmp.path(), "wgenty-code").unwrap();
        assert_eq!(found, exact);
    }

    #[test]
    fn find_in_dir_falls_back_to_triple_suffix() {
        let tmp = tempfile::tempdir().unwrap();
        let suffixed = tmp.path().join("wgenty-code-aarch64-apple-darwin");
        // A decoy shell binary + an unrelated resource share the prefix.
        std::fs::write(tmp.path().join("wgenty-code-desktop"), b"shell").unwrap();
        std::fs::write(tmp.path().join("wgenty-code.icns"), b"icns").unwrap();
        std::fs::write(&suffixed, b"daemon").unwrap();

        let found = find_daemon_in_dir(tmp.path(), "wgenty-code").unwrap();
        assert_eq!(found, suffixed);
    }

    #[test]
    fn find_in_dir_returns_none_when_absent() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("wgenty-code-desktop"), b"shell").unwrap();
        assert!(find_daemon_in_dir(tmp.path(), "wgenty-code").is_none());
    }
}
