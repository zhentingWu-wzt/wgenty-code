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
    let deadline = std::time::Instant::now() + SHUTDOWN_CONFIRM_TIMEOUT;
    loop {
        if crate::utils::http::probe_daemon_health(&client, &base_url)
            .await
            .is_none()
        {
            println!("Daemon on 127.0.0.1:{port} stopped.");
            return Ok(());
        }
        if std::time::Instant::now() > deadline {
            println!(
                "Shutdown requested, but the daemon is still answering after {}s.",
                SHUTDOWN_CONFIRM_TIMEOUT.as_secs()
            );
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
}
