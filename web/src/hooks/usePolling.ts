import { useEffect, useRef } from "react";

/**
 * Poll an async function on a fixed interval while `active` is true.
 *
 * Calls `fn` immediately on activation, then every `intervalMs`. Stops cleanly
 * on unmount or when `active` flips false. Errors are swallowed (logged) so a
 * transient daemon hiccup doesn't kill the poller — the next tick retries.
 *
 * Used by the side panels (todos / tasks / pending-permissions) which mirror
 * the TUI's polling model (src/tui/agent/adapters.rs:152-235 polls every 500ms).
 */
export function usePolling(
  fn: () => Promise<void>,
  active: boolean,
  intervalMs: number,
): void {
  const fnRef = useRef(fn);
  fnRef.current = fn;

  useEffect(() => {
    if (!active) return;
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | null = null;

    const tick = async () => {
      try {
        await fnRef.current();
      } catch (err) {
        // Swallow — polling must survive transient errors.
        console.warn("[usePolling] tick failed", err);
      } finally {
        if (!cancelled) {
          timer = setTimeout(tick, intervalMs);
        }
      }
    };

    tick();
    return () => {
      cancelled = true;
      if (timer) clearTimeout(timer);
    };
  }, [active, intervalMs]);
}
