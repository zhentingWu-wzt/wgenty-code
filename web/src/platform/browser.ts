/**
 * Browser implementation of PlatformCapability.
 *
 * Used when the app runs in a browser (Vite dev server or static host). The
 * daemon must be started separately by the user — `ensureDaemon` is a no-op.
 * File operations use the browser's limited File API (no direct filesystem
 * access).
 */
import type { PlatformCapability } from "./types";

export const browserPlatform: PlatformCapability = {
  name: "browser",

  // No-op: the browser cannot start a process. The user must run
  // `wgenty-code daemon` separately. Health-check polling in App.tsx will
  // report "disconnected" until they do.
  async ensureDaemon() {
    /* no-op in browser */
  },

  // Wrap beforeunload so callers don't need to know the browser event name.
  onBeforeClose(handler: () => void): () => void {
    const listener = (e: BeforeUnloadEvent) => {
      handler();
      // The app already manages its own preventDefault logic in App.tsx;
      // we only forward the notification.
      void e;
    };
    window.addEventListener("beforeunload", listener);
    return () => window.removeEventListener("beforeunload", listener);
  },

  // Browser directory picker via the non-standard webkitdirectory attribute.
  // This is a degraded experience (no real path, just relative file entries),
  // so for now we return null and let the UI fall back to manual text input.
  // Tauri provides a real native dialog via pickDirectory in platform-tauri.js.
  async pickDirectory(): Promise<string | null> {
    return null;
  },
};
