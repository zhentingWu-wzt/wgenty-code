/**
 * Platform detection and singleton accessor.
 *
 * The Tauri initialization script (desktop/src/platform-tauri.js) sets
 * `window.__wgentyPlatform` before React boots. If present, we use it.
 * Otherwise we fall back to the browser implementation.
 *
 * Usage in app code:
 *   import { getPlatform } from "@/platform";
 *   const platform = getPlatform();
 *   await platform.ensureDaemon?.();
 */
import { browserPlatform } from "./browser";
import type { PlatformCapability } from "./types";

let cached: PlatformCapability | null = null;

/**
 * Get the active platform capability. Memoized after the first call — the
 * runtime doesn't change during the app's lifetime.
 */
export function getPlatform(): PlatformCapability {
  if (cached) return cached;
  // The Tauri init script injects this before page scripts run. If it's
  // absent, we're in a plain browser.
  cached = window.__wgentyPlatform ?? browserPlatform;
  return cached;
}

/** Reset the cache (test-only — lets unit tests swap platforms). */
export function _resetPlatformCacheForTests(): void {
  cached = null;
}

export type { PlatformCapability, PlatformName } from "./types";
