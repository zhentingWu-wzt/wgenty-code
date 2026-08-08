//! Tauri desktop shell (spike) for the wgenty-code web frontend.
//!
//! Architecture: the existing `web/` React app is a thin client over the
//! wgenty-code daemon (loopback HTTP + SSE). In dev, Vite's dev-proxy injects
//! the daemon bearer token so the browser never sees it. Tauri has no such
//! proxy, so we reproduce the same property with an **initialization script**
//! (see `desktop/src/token-injection.js`): before the web app boots, it
//! monkey-patches `window.fetch` so every `/api/*` request gets an
//! `Authorization: Bearer <token>` header injected by the host. The web/ source
//! therefore needs zero changes — `DaemonClient`'s `fetch("/api/v1/...")` keeps
//! working unchanged.
//!
//! This mirrors the security model of the Vite proxy: the token is read by the
//! host process (not visible in webview JS source on first load), and only
//! applied to loopback `/api/*` requests. The daemon binds 127.0.0.1 only.

mod daemon_manager;

use tauri::Manager;

/// The token-injection script source, embedded at compile time.
///
/// Contains a `__WGENTY_TOKEN__` placeholder that we replace with the
/// eagerly-read token (a JS string literal or `null`) before injecting.
const TOKEN_INJECTION_TEMPLATE: &str = include_str!("../../src/token-injection.js");

/// The Tauri platform implementation, embedded at compile time.
///
/// Sets `window.__wgentyPlatform` before React boots, picked up by
/// `web/src/platform/index.ts`. Separate from token injection (transparent
/// auth) — this exposes explicit platform capabilities (ensureDaemon, etc.).
const PLATFORM_TAURI_SCRIPT: &str = include_str!("../../src/platform-tauri.js");

/// Tauri command: the frontend's fetch wrapper calls this on a 401 to refresh
/// the token (daemon may have restarted and rotated it). Kept uncached because
/// reads are cheap and token rotation is the whole point.
#[tauri::command]
fn read_daemon_token() -> Option<String> {
    daemon_manager::read_token()
}

/// Tauri command: ensure the daemon is running and reachable.
///
/// Delegates to `daemon_manager::ensure_daemon`: discovers a healthy running
/// instance first (via `~/.wgenty-code/daemon.json` + heartbeat check); if
/// none found, spawns `wgenty-code daemon` as a separate process and waits for
/// it to become reachable. The daemon is **detached** — it survives the Tauri
/// shell exit so other UIs (TUI, browser) can keep using it.
///
/// In a packaged app, the daemon binary is bundled as a Tauri resource
/// (externalBin in tauri.conf.json); the resource_dir is resolved from the
/// AppHandle so `locate_daemon_binary` can find it.
///
/// Guards against double-invoke (React StrictMode runs effects twice in dev):
/// the first call does the work; concurrent/subsequent calls await the same
/// result via a shared `OnceCell`.
static DAEMON_ENSURE: tokio::sync::OnceCell<Result<(), String>> =
    tokio::sync::OnceCell::const_new();

#[tauri::command]
async fn ensure_daemon(app: tauri::AppHandle) -> Result<(), String> {
    // Resolve the resource directory for packaged-binary lookup.
    // In dev mode, this returns a path inside target/ — harmless, as
    // locate_daemon_binary falls back to the target/{debug,release}/ check.
    let resource_dir = app.path().resource_dir().ok();
    DAEMON_ENSURE
        .get_or_init(|| async {
            daemon_manager::ensure_daemon(resource_dir)
                .await
                .map(|_| ())
        })
        .await
        .clone()
}

/// Read the token eagerly (host-side) for embedding into the init script.
///
/// Thin wrapper around `daemon_manager::read_token` so `build_injection_script`
/// stays in this file (it needs the Tauri-specific string escaping).
fn read_token_eager() -> Option<String> {
    daemon_manager::read_token()
}

/// Build the initialization script with the eagerly-known token embedded.
///
/// If the daemon is already running, the token is baked in so the very first
/// `/api/*` request is authenticated without a round-trip. If not, the
/// placeholder becomes `null` and the script falls back to the IPC command
/// (with a 401-retry path) once the daemon comes up.
fn build_injection_script() -> String {
    let token_literal = match read_token_eager() {
        Some(t) => format!("\"{}\"", t.replace('\\', "\\\\").replace('"', "\\\"")),
        None => "null".to_string(),
    };
    TOKEN_INJECTION_TEMPLATE.replace("__WGENTY_TOKEN__", &token_literal)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Serve the app over http://localhost:PORT in production (not the default
    // tauri://localhost custom protocol) so the webview origin stays in the
    // daemon's CORS allow-list. Port 5173 mirrors the dev workflow (Vite dev
    // server) and is already whitelisted by the daemon.
    const LOCALHOST_PORT: u16 = 5173;

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_localhost::Builder::new(LOCALHOST_PORT).build())
        .setup(move |app| {
            // Build the main window ourselves (instead of declaring it in
            // tauri.conf.json) so we can attach the initialization script via
            // the builder — the only API surface that injects JS *before* the
            // page's own scripts run.
            //
            // In dev, Tauri loads the devUrl from tauri.conf.json (the Vite
            // dev server). In production, the localhost plugin serves the
            // frontendDist assets at http://localhost:5173, so we point the
            // window there explicitly.
            let url = format!("http://localhost:{}", LOCALHOST_PORT)
                .parse()
                .expect("valid localhost URL");
            let _window = tauri::webview::WebviewWindowBuilder::new(
                app,
                "main",
                tauri::WebviewUrl::External(url),
            )
            .title("wgenty-code")
            .inner_size(1200.0, 800.0)
            .min_inner_size(800.0, 500.0)
            .initialization_script(build_injection_script())
            // Second init script: exposes window.__wgentyPlatform (explicit
            // platform capabilities) before React boots. Separate from token
            // injection (transparent auth).
            .initialization_script(PLATFORM_TAURI_SCRIPT.to_string())
            .build()?;

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![read_daemon_token, ensure_daemon])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
