/**
 * Tauri platform implementation — injected before React boots.
 *
 * This file is embedded into the Tauri binary (include_str! in lib.rs) and
 * registered as a second initialization script (alongside token-injection.js).
 * It sets `window.__wgentyPlatform`, which web/src/platform/index.ts picks up
 * via `getPlatform()`.
 *
 * Capabilities implemented here:
 * - name: "desktop"
 * - ensureDaemon: invokes the Rust `ensure_daemon` command (spawns or reuses)
 * - onBeforeClose: stub for now (full impl needs Tauri window event plugin)
 * - pickDirectory: stub for now (needs tauri-plugin-dialog)
 *
 * Token injection is NOT handled here — it lives in token-injection.js, which
 * patches window.fetch. This separation keeps auth (transparent) distinct from
 * capabilities (explicit platform interface).
 */
(function () {
  if (window.__wgentyPlatform) return; // idempotent

  var invoke = window.__TAURI__ && window.__TAURI__.core && window.__TAURI__.core.invoke;
  if (!invoke) {
    // If Tauri's IPC bridge isn't available, don't set __wgentyPlatform — the
    // app will fall back to browserPlatform. This makes the script safe to
    // load even outside Tauri (e.g. during Vite dev without Tauri running).
    return;
  }

  window.__wgentyPlatform = {
    name: "desktop",

    /**
     * Ask the Rust host to ensure the daemon is running. The host either
     * discovers a running instance or spawns one in-process. Resolves once
     * the daemon health check passes.
     */
    ensureDaemon: function () {
      return invoke("ensure_daemon");
    },

    /**
     * Window close hook. For now this is a placeholder — the real
     * implementation will use tauri's window close-requested event to run
     * cleanup (disconnect SSE, shut down embedded daemon) before the window
     * closes. Returns an unsubscribe stub.
     */
    onBeforeClose: function (handler) {
      // TODO(foundation): wire to Tauri window 'close-requested' event.
      // For now, fall back to beforeunload (works in webview too).
      var listener = function () {
        try {
          handler();
        } catch (e) {
          console.error("onBeforeClose handler error:", e);
        }
      };
      window.addEventListener("beforeunload", listener);
      return function () {
        window.removeEventListener("beforeunload", listener);
      };
    },

    /**
     * Native directory picker. Placeholder until tauri-plugin-dialog is added.
     * Returns null so the UI falls back to manual path entry.
     */
    pickDirectory: function () {
      // TODO(foundation): return invoke("pick_directory") once the dialog
      // plugin is wired up.
      return Promise.resolve(null);
    },
  };
})();
