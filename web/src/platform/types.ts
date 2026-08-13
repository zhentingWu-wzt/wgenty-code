/**
 * Platform abstraction layer — isolates browser vs desktop (Tauri) differences.
 *
 * The web/ React app is shared between two runtimes:
 * - **Browser**: served by Vite dev server (or any static host). Token injected
 *   by the Vite proxy. Cannot start processes, use native dialogs, etc.
 * - **Tauri desktop**: the same app loaded in a webview. Token injected by the
 *   Rust host's initialization script. CAN start the daemon, use native file
 *   dialogs, hook window-close events, etc.
 *
 * This interface is the single seam between the app and the runtime. App code
 * calls `getPlatform().xxx()` — never `if (isTauri)`. Each runtime provides its
 * own implementation (browser.ts here, platform-tauri.js in desktop/). The
 * Tauri implementation is injected via `window.__wgentyPlatform` before React
 * boots; if absent, the browser implementation is used.
 *
 * Design principle: **DaemonClient is NOT part of this interface.** Token
 * injection stays external (Vite proxy / Tauri init script) so the existing
 * `fetch("/api/v1/...")` code is unchanged in both runtimes. This interface
 * only covers capabilities that genuinely differ between browser and desktop.
 */

/** Identifies which runtime the app is currently executing in. */
export type PlatformName = "browser" | "desktop";

/**
 * Platform-specific capabilities. All methods are optional — a runtime
 * implements only what it can. Callers use optional chaining
 * (`platform.ensureDaemon?.()`) so absent capabilities are silent no-ops.
 */
export interface PlatformCapability {
  /** The runtime name. */
  readonly name: PlatformName;

  /**
   * Ensure the daemon is running. In the browser this is a no-op (the user is
   * responsible for starting it). In Tauri, the Rust host spawns/reuses the
   * daemon before the first API call. Resolves once the daemon health check
   * passes; rejects if the daemon cannot be started.
   */
  ensureDaemon?(): Promise<void>;

  /**
   * Register a handler that runs before the window/tab closes. Returns an
   * unsubscribe function.
   *
   * - Browser: wraps `window.addEventListener('beforeunload', ...)`.
   * - Tauri: hooks the window close-requested event (can delay or prevent it).
   */
  onBeforeClose?(handler: () => void): () => void;

  /**
   * Open a native directory picker. Returns the selected path or null if
   * cancelled. In the browser, falls back to `<input webkitdirectory>` or
   * returns null if unavailable.
   */
  pickDirectory?(): Promise<string | null>;

  /**
   * Read a file as text. In the browser this uses the File API; in Tauri it
   * reads from the real filesystem via IPC. Currently unused — reserved for
   * future features that need direct file access (e.g. loading a config file).
   */
  readFile?(path: string): Promise<string>;
}

/**
 * Global marker injected by the Tauri initialization script. When present,
 * `getPlatform()` returns this object instead of the browser fallback.
 *
 * Declared on the Window interface so TypeScript recognizes it without casts.
 */
declare global {
  interface Window {
    __wgentyPlatform?: PlatformCapability;
  }
}
