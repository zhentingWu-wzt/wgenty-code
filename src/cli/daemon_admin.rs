//! `wgenty-code daemon status|stop` — inspect and control the running daemon.
//!
//! Both subcommands also work in builds without the `daemon` feature: they
//! only read the discovery/token files and talk HTTP to an already-running
//! daemon process.

use anyhow::Context;

/// Timeout for admin requests against the localhost daemon.
const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);
/// How long `stop` waits for the daemon to actually go down before giving up.
const SHUTDOWN_CONFIRM_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

fn admin_client() -> anyhow::Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(REQUEST_TIMEOUT)
        .build()
        .context("build daemon admin HTTP client")
}

/// Effective port: the discovery file (written by the running daemon) wins;
/// otherwise fall back to the CLI default / explicit `--port`.
fn effective_port(cli_port: u16) -> (u16, Option<crate::utils::discovery::DiscoveryFile>) {
    let discovery = crate::utils::discovery::read_discovery_file();
    let port = discovery.as_ref().map(|d| d.port).unwrap_or(cli_port);
    (port, discovery)
}

/// `wgenty-code daemon status` — report discovery-file state and probe the
/// public health endpoint.
pub async fn status(cli_port: u16) -> anyhow::Result<()> {
    let (port, discovery) = effective_port(cli_port);
    let base_url = format!("http://127.0.0.1:{port}");

    match &discovery {
        Some(file) => {
            let age = chrono::Utc::now().signed_duration_since(file.heartbeat_at);
            let fresh = age.num_seconds() <= crate::utils::discovery::HEARTBEAT_STALE_SECS as i64;
            let token_matches = crate::utils::read_daemon_token()
                .map(|t| t == file.token)
                .unwrap_or(false);
            println!(
                "Discovery file: {}",
                crate::utils::discovery::discovery_file_path().display()
            );
            println!("  Port:       {}", file.port);
            println!("  PID:        {}", file.pid);
            println!("  Started:    {}", file.started_at);
            println!(
                "  Heartbeat:  {}s ago ({})",
                age.num_seconds(),
                if fresh { "fresh" } else { "stale" }
            );
            // Never print the token itself — only whether the two on-disk
            // copies agree (a mismatch means a stale token is on disk).
            println!(
                "  Token file: {}",
                if token_matches {
                    "matches"
                } else {
                    "MISMATCH (stale token on disk)"
                }
            );
        }
        None => println!("No discovery file found; probing port {port} anyway."),
    }

    let client = admin_client()?;
    match crate::utils::http::probe_daemon_health(&client, &base_url).await {
        Some(health) => {
            println!(
                "Health:       {} (version {})",
                health.status, health.version
            );
            println!("Daemon is running at {base_url}");
        }
        None => {
            println!("Health:       no wgenty daemon answering on 127.0.0.1:{port}");
            if discovery.is_some() {
                println!(
                    "The daemon appears to be down; the discovery file above is stale and \
                     can be removed."
                );
            }
        }
    }
    Ok(())
}

/// `wgenty-code daemon stop` — ask the daemon for a graceful shutdown via its
/// authenticated shutdown endpoint, then wait until it stops answering.
pub async fn stop(cli_port: u16) -> anyhow::Result<()> {
    let (port, discovery) = effective_port(cli_port);
    let base_url = format!("http://127.0.0.1:{port}");
    let token = crate::utils::read_daemon_token().with_context(|| {
        format!(
            "no daemon token found at {}; is a daemon running?",
            crate::utils::daemon_token_path().display()
        )
    })?;

    let client = admin_client()?;
    let resp = client
        .post(format!("{base_url}/api/v1/shutdown"))
        .bearer_auth(&token)
        .send()
        .await
        .with_context(|| format!("no daemon responding on 127.0.0.1:{port}"))?;

    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        // The running daemon holds a different token than the one on disk —
        // it was restarted while a stale token file survived. The API path
        // cannot reach it, so point the user at the PID instead.
        let hint = match &discovery {
            Some(file) => format!("kill it directly with `kill {}`", file.pid),
            None => "kill the process listening on the port".to_string(),
        };
        anyhow::bail!(
            "daemon rejected the local token (401); it was likely restarted with a fresh token. \
             To stop it, {hint}, then remove {}.",
            crate::utils::daemon_token_path().display()
        );
    }
    if !resp.status().is_success() {
        anyhow::bail!("shutdown request failed ({})", resp.status());
    }

    println!("Shutdown requested; waiting for the daemon to exit...");
    wait_until_down(&client, &base_url, SHUTDOWN_CONFIRM_TIMEOUT).await;
    Ok(())
}

/// Poll until the daemon stops answering health probes (or the timeout
/// elapses). Shared by `stop` and `kill_predecessor`.
async fn wait_until_down(client: &reqwest::Client, base_url: &str, timeout: std::time::Duration) {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if crate::utils::http::probe_daemon_health(client, base_url)
            .await
            .is_none()
        {
            println!("Daemon on {base_url} stopped.");
            return;
        }
        if std::time::Instant::now() > deadline {
            println!(
                "Daemon on {base_url} is still answering after {}s.",
                timeout.as_secs()
            );
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
}

/// Pre-start step for `wgenty-code daemon`: if a previous daemon is still
/// healthy on its discovered port, stop it first so every launch runs the
/// current binary (a stale long-lived daemon otherwise keeps serving old
/// code). Tries the graceful shutdown API, then escalates to SIGTERM and
/// SIGKILL by PID. A stale discovery file (nothing answering) is a no-op.
pub async fn kill_predecessor() -> anyhow::Result<()> {
    let Some(file) = crate::utils::discovery::read_discovery_file() else {
        return Ok(());
    };
    let base_url = format!("http://127.0.0.1:{}", file.port);
    let client = admin_client()?;
    if crate::utils::http::probe_daemon_health(&client, &base_url)
        .await
        .is_none()
    {
        return Ok(()); // stale discovery file; nothing is running
    }
    if file.pid == std::process::id() {
        return Ok(()); // defensive: never kill ourselves
    }

    println!(
        "Stopping previous daemon (pid {}, port {}) before restart...",
        file.pid, file.port
    );

    // 1. Graceful: the old daemon removes its token/discovery files itself.
    // "Down" must mean the PID is gone, not just that the port stopped
    // answering: a daemon in graceful shutdown stops accepting immediately
    // but may still be draining long-lived SSE streams — declaring success
    // here on the health probe alone is what let zombie daemons accumulate.
    let _ = client
        .post(format!("{base_url}/api/v1/shutdown"))
        .bearer_auth(&file.token)
        .send()
        .await;
    wait_until_down_quiet(&client, &base_url, std::time::Duration::from_secs(6)).await;
    if crate::utils::http::probe_daemon_health(&client, &base_url)
        .await
        .is_none()
        && !process_alive(file.pid)
    {
        println!("Previous daemon stopped.");
        return Ok(());
    }

    // 2. SIGTERM, then SIGKILL (unresponsive pre-shutdown-endpoint builds).
    terminate_process(file.pid, false);
    wait_until_down_quiet(&client, &base_url, std::time::Duration::from_secs(3)).await;
    if crate::utils::http::probe_daemon_health(&client, &base_url)
        .await
        .is_none()
        && !process_alive(file.pid)
    {
        println!("Previous daemon terminated.");
        return Ok(());
    }

    terminate_process(file.pid, true);
    wait_until_down_quiet(&client, &base_url, std::time::Duration::from_secs(2)).await;
    if crate::utils::http::probe_daemon_health(&client, &base_url)
        .await
        .is_some()
    {
        anyhow::bail!(
            "failed to stop previous daemon (pid {}); port {} is still in use",
            file.pid,
            file.port
        );
    }
    // A force-killed daemon could not clean up its state files; drop them so
    // the new instance does not inherit a stale token/discovery pair. The
    // ownership checks guard the narrow race where a fresh daemon already
    // started (and rewrote the files) between our kill and this cleanup.
    let _ = crate::utils::remove_daemon_token_if_matches(&file.token);
    let _ = crate::utils::discovery::remove_discovery_file_if_pid(file.pid);
    println!("Previous daemon killed.");
    Ok(())
}

/// Like [`wait_until_down`] but silent (escalation steps report their own
/// final outcome).
async fn wait_until_down_quiet(
    client: &reqwest::Client,
    base_url: &str,
    timeout: std::time::Duration,
) {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() <= deadline {
        if crate::utils::http::probe_daemon_health(client, base_url)
            .await
            .is_none()
        {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
}

/// Check whether `pid` still exists (sends no real signal).
#[cfg(unix)]
fn process_alive(pid: u32) -> bool {
    std::process::Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[cfg(windows)]
fn process_alive(pid: u32) -> bool {
    // tasklist exits 0 even when nothing matches; inspect the output.
    let pid = pid.to_string();
    std::process::Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/NH"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains(&pid))
        .unwrap_or(false)
}

/// Send SIGTERM (`force = false`) or SIGKILL (`force = true`) to `pid`.
#[cfg(unix)]
fn terminate_process(pid: u32, force: bool) {
    let sig = if force { "-9" } else { "-TERM" };
    let _ = std::process::Command::new("kill")
        .arg(sig)
        .arg(pid.to_string())
        .status();
}

/// Windows has no signal granularity; `taskkill /F` is the only option.
#[cfg(windows)]
fn terminate_process(pid: u32, force: bool) {
    let pid = pid.to_string();
    let mut args = vec!["/PID", pid.as_str(), "/T"];
    if force {
        args.push("/F");
    }
    let _ = std::process::Command::new("taskkill").args(args).status();
}
