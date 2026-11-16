import { useSyncExternalStore } from "react";
import { selectPendingApprovalCount, useSessionManager } from "../state/sessionManager";
import type { DaemonClient } from "../api/client";
import { useWorkspaceRoot } from "../features/files/useWorkspaceRoot";

/** 底部状态栏：daemon 连接 · 待审批数 · workspace · context/cache。
 *  轮次 phase 在 TurnStatus，模型与权限模式在 Composer 上方的 chips。 */
export function StatusBar({ client }: { client: DaemonClient }) {
  const connection = useSessionManager((s) => s.connection);
  const contextWindow = useSessionManager((s) => s.contextWindow);
  const activeStore = useSessionManager((s) =>
    s.activeId ? (s.entries[s.activeId]?.store ?? null) : null,
  );
  const pendingApprovals = useSessionManager(selectPendingApprovalCount);
  // Workspace root of the ACTIVE session (worktree, else project path, else
  // the main project) — shows which checkout a turn will land in.
  const { root: workRoot } = useWorkspaceRoot(client);

  // Context-window occupancy of the active session, updated LIVE by
  // usage_update events (after every LLM call) and re-synced by the turn-end
  // turn_context snapshot. The per-session store lives outside React context
  // here (StatusBar sits above the provider), so subscribe directly — the
  // primitive snapshot re-renders only on real usage changes, and switching
  // sessions re-subscribes via the new store's subscribe identity.
  const contextTokens = useSyncExternalStore(
    activeStore?.subscribe ?? noopSubscribe,
    () => activeStore?.getState().contextTokens ?? null,
  );
  // Prompt-cache hit of the last LLM call (turn_context snapshot + live
  // usage_update events). Null = gateway doesn't report it → hide the badge.
  const cachedTokens = useSyncExternalStore(
    activeStore?.subscribe ?? noopSubscribe,
    () => activeStore?.getState().cachedTokens ?? null,
  );
  // Session-lifetime cache economy — the weighted hit rate Σcached/Σprompt
  // recomputed server-side after every call. Meaningful average (not the
  // mean of per-call percentages); null before the first usage report.
  const sessionCacheTotals = useSyncExternalStore(
    activeStore?.subscribe ?? noopSubscribe,
    () => activeStore?.getState().sessionCacheTotals ?? null,
  );
  const cacheHitPct =
    sessionCacheTotals && sessionCacheTotals.prompt_tokens > 0
      ? Math.round(
          Math.min(sessionCacheTotals.cached_tokens / sessionCacheTotals.prompt_tokens, 1) * 100,
        )
      : null;

  const statusText =
    connection === "connected"
      ? "online"
      : connection === "disconnected"
        ? "offline"
        : "connecting";

  return (
    <footer className="flex h-6 shrink-0 items-center gap-3 border-t border-border bg-background px-3 text-[11px] text-muted-foreground">
      <span className="flex items-center gap-1.5">
        <span
          className={
            connection === "connected"
              ? "h-1.5 w-1.5 rounded-full bg-success"
              : connection === "disconnected"
                ? "h-1.5 w-1.5 rounded-full bg-danger"
                : "h-1.5 w-1.5 rounded-full bg-warning"
          }
        />
        {statusText}
      </span>
      {pendingApprovals > 0 && (
        <span className="rounded-sm bg-warning/20 px-1 text-warning">
          {pendingApprovals} approval{pendingApprovals > 1 ? "s" : ""}
        </span>
      )}
      {workRoot && (
        // Full worktree paths overflow the narrow phone status bar (<md);
        // desktop keeps the full path with a hover tooltip.
        <span className="shrink-0 font-mono max-md:hidden" title={workRoot}>
          {workRoot}
        </span>
      )}
      <div className="flex-1" />
      {/* Context-usage bar — a real CSS track/fill bar (▓/░ shade glyphs read
          as a solid block in web fonts, hiding the fill level). Fill color
          mirrors the TUI context_bar: green < 50%, yellow 50–80%, red ≥ 80%.
          Hidden until the first turn reports usage or the window is unknown. */}
      {contextWindow !== null && contextWindow > 0 && contextTokens !== null && (
        <span
          className="flex items-center gap-1.5"
          title={`context ${contextTokens.toLocaleString()} / ${contextWindow.toLocaleString()} tokens`}
        >
          <span className="h-1.5 w-14 overflow-hidden rounded-full bg-muted">
            <span
              className={
                contextTokens / contextWindow >= 0.8
                  ? "block h-full bg-danger"
                  : contextTokens / contextWindow >= 0.5
                    ? "block h-full bg-warning"
                    : "block h-full bg-success"
              }
              style={{
                width: `${Math.round(Math.min(contextTokens / contextWindow, 1) * 100)}%`,
              }}
            />
          </span>
          <span className="tabular-nums">
            {Math.round(Math.min(contextTokens / contextWindow, 1) * 100)}%
          </span>
        </span>
      )}
      {/* Cache badge — session-lifetime provider prompt-cache hit rate,
          token-weighted (Σ cached / Σ prompt, server-recomputed per call).
          Hidden until the daemon reports usage. CSS tooltip (group-hover)
          instead of the native title attr: WKWebView/Tauri never renders
          native tooltips, and even browsers delay them ~1s. */}
      {cacheHitPct !== null && (
        <span className="group relative shrink-0">
          <span className="rounded-sm bg-success/15 px-1 tabular-nums text-success">
            cache {cacheHitPct}%
          </span>
          {sessionCacheTotals && (
            <span className="pointer-events-none absolute bottom-full right-0 z-50 mb-1 hidden whitespace-nowrap rounded-md border border-border bg-popover px-2 py-1 text-left text-[11px] text-foreground shadow-lg group-hover:block">
              session cache: {sessionCacheTotals.cached_tokens.toLocaleString()} /{" "}
              {sessionCacheTotals.prompt_tokens.toLocaleString()} prompt tokens (weighted avg)
              <br />
              last call:{" "}
              {cachedTokens !== null ? `${cachedTokens.toLocaleString()} cached` : "no report"}
            </span>
          )}
        </span>
      )}
    </footer>
  );
}

const noopSubscribe = () => () => {};
