# Proposal: Startup Render-First Optimization

## Summary

Optimize the time from process launch to first TUI frame render. Currently every
blocking initialization step (daemon startup, MCP connections, skill discovery,
plugin loading, AutoDream consolidation, memory recall) runs **serially before**
the first `terminal.draw`. This change restructures the startup sequence so the
terminal enters and renders a splash frame immediately, then performs heavy
initialization concurrently in the background.

## Motivation

Tracing the startup path (`main.rs` -> `run_repl` -> `App::run` -> first
`terminal.draw` at `src/tui/app/mod.rs:632`), all of the following execute
serially before the user sees anything:

1. `auto_install()` — synchronous bundled-skills disk write (`cli/args.rs:118`)
2. `start_daemon().await` — **blocks terminal entry** (`cli/args.rs:140`):
   - `DaemonState::new`: `SkillLoader` fs traversal + **serial** MCP server
     connections (each up to 15s timeout) + `session_manager.load_all()` (every
     persisted session JSON)
   - health-readiness poll: up to 50 × 100ms
3. `App::new()` — **duplicates** `SkillLoader::load_from_dirs` already done in
   the daemon, plus `ExternalSkillRegistry::discover` multi-root traversal, plus
   `workflow.yaml` read (`app/mod.rs:202`)
4. `PluginManager.load_all().await` (`args.rs:187`)
5. `App::run()` pre-render: `auto_dream.check_and_run()` (an LLM-backed
   consolidation when the gate passes — enabled by default, gate passes by
   default) + `memory.load()` + `search_memories()` (`app/mod.rs:548-610`)

AutoDream is `enabled: true` by default and `ConsolidationState::default()` sets
`last_consolidated_at = now - (min_hours + 1)`, so the time gate passes on every
fresh launch; with ≥5 recent sessions the LLM consolidation runs synchronously
before the first frame.

## Approach (confirmed: Approach A — render-first)

Enter the terminal and paint a splash frame **before** daemon readiness, then
perform heavy init concurrently:

- Enter alternate screen + raw mode first; render a lightweight splash.
- `auto_install()` → `spawn_blocking` after terminal entry (skip via
  `is_auto_installed()` fast-path).
- `start_daemon` → background: bind listener (fast), spawn `axum::serve`, return
  base_url immediately. Defer `DaemonState::new` heavy work (MCP connect, session
  load) to background tasks; daemon health passes on bind, not on full init.
- MCP servers → connect **concurrently** (`join_all`) in the background, not
  serially; tools register as each server comes online.
- `App::new` skill discovery → run concurrently with daemon startup; deduplicate
  against the daemon-loaded inventory (fetch via daemon API or share).
- `App::run` → first `terminal.draw` **immediately**, then `tokio::spawn`
  AutoDream + memory recall; inject recalled memories via event when ready.
- `session_manager.load_all()` → load metadata index only; full session detail
  loaded lazily on access.

## Non-goals

- No change to user-facing TUI behavior or command set — only latency.
- No change to the daemon HTTP API contract (existing endpoints unchanged; MCP
  tools may register slightly later but are available before first user turn).
- No new features/capabilities.

## Scope

Single cohesive change `startup-render-first`. Six implementation tasks sharing
the startup path, verified together. Touches: `cli/args.rs`, `tui/util.rs`,
`tui/app/mod.rs`, `mcp/mod.rs`, `daemon/state.rs`, `knowledge/embedded.rs`.

## Risk

- `run_repl` startup-sequence restructure (medium) — splash must restore terminal
  on error before `App` exists; covered by existing panic hook + a splash-scoped
  cleanup.
- MCP concurrent connect — each connection already has its own 15s timeout and
  degrades gracefully; concurrency only changes ordering, not failure semantics.
- AutoDream/memory deferred — recalled memories arrive one frame later; injected
  via event, no behavioral regression.
