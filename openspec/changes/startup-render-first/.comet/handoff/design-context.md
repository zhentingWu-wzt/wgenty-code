# Comet Design Handoff

- Change: startup-render-first
- Phase: design
- Mode: compact
- Context hash: e4e730b5f79bbb6abee7fab2a122376e1aa17050979d1cc38207272804e9fa51

Generated-by: comet-handoff.sh

OpenSpec remains the canonical capability spec. This handoff is a deterministic, source-traceable context pack, not an agent-authored summary.

## openspec/changes/startup-render-first/proposal.md

- Source: openspec/changes/startup-render-first/proposal.md
- Lines: 1-78
- SHA256: 035cf51701b416b4addefcdaf66f196bbf5e9b2e50253b08383a63fb1659df2b

```md
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
```

## openspec/changes/startup-render-first/design.md

- Source: openspec/changes/startup-render-first/design.md
- Lines: 1-125
- SHA256: 31efcf53e31e84112067837644fc9d86212d8a613562a10ba2a5827d8f41ef5f

[TRUNCATED]

```md
# Design: Startup Render-First Optimization

> Status: **draft** (phase: open). Refined into a Design Doc + delta spec during
> `/comet-design`. This captures the confirmed architecture (Approach A).

## Current startup sequence (problem)

```
main()
  Settings::load()                         # cheap
  run_repl()
    auto_install()                         # ① sync disk write  [BLOCKS]
    start_daemon().await                   # ② BLOCKS terminal entry
      DaemonState::new().await
        SkillLoader::load_from_dirs        #   fs traversal
        mcp.connect_configured_tools       #   SERIAL, each ≤15s
        session_manager.load_all()         #   all session JSON
      health poll 50×100ms
    EnterAlternateScreen / raw_mode        # terminal entered HERE
    App::new()                             # ③ dup skill discovery + ext registry
    PluginManager.load_all().await         # ④
    App::run()
      auto_dream.check_and_run().await     # ⑤ LLM call if gate passes [BLOCKS]
      memory.load() + search_memories()    #     [BLOCKS]
      terminal.draw  ← FIRST RENDER
```

User sees nothing until ①-⑤ all complete serially.

## Target startup sequence (Approach A - render-first)

```
main()
  Settings::load()
  run_repl()
    EnterAlternateScreen / raw_mode        # terminal entered FIRST
    render splash frame                    # FIRST RENDER (~instant)
    spawn_blocking(auto_install)           # ① background, skip if installed
    start_daemon_bg() -> (base_url, ready_rx)
      TcpListener::bind                    #   fast
      spawn axum::serve
      spawn DaemonState::new_heavy()       #   MCP concurrent + session lazy -> bg
      send ready signal on bind            #   health passes immediately
    App::new()  (concurrent w/ daemon)     # ③ skill discovery concurrent
      // skill inventory deduped via daemon API or shared loader
    poll ready_rx; refresh splash          # ② non-blocking, renders progress
    on ready: App::run()
      terminal.draw  ← IMMEDIATE           # first real frame
      spawn(auto_dream.check_and_run)      # ⑤ background -> event inject
      spawn(memory.load + recall)          #     background -> event inject
    PluginManager.load_all -> spawn        # ④ background, merge via event
```

First render drops from "all init done" to "terminal switch" (~tens of ms).

## Component changes

### 1. `run_repl` restructure (`src/cli/args.rs`)
Split terminal setup from daemon startup. Order becomes:
enter-terminal -> splash -> daemon-bg -> ready -> App. The splash renders a
simple "Starting wgenty-code…" frame using a minimal ratatui draw (no full App).
On error before `App` exists, restore terminal (disable raw mode, leave
alternate screen) - reuse the existing panic-hook pattern scoped to the splash.

### 2. `start_daemon` -> `start_daemon_bg` (`src/tui/util.rs`)
- Bind `TcpListener` (already synchronous-ready).
- Construct `DaemonState` but split heavy init out of `DaemonState::new`:
  - `DaemonState::new_shell()` - cheap shell (coordinator, policy, registries)
    constructed synchronously.
  - `DaemonState::init_heavy()` - MCP connect (concurrent), session metadata
    index - spawned as a background task; the daemon serves immediately with
    the shell state and gains tools/sessions as they load.
- Return `(base_url, ready_rx)` immediately after `axum::serve` spawn. Drop the
  50×100ms health poll (the listener is bound; connections succeed at once).

### 3. MCP concurrent connect (`src/mcp/mod.rs`, `src/daemon/state.rs`)
Replace the `for config in ... { connect_server().await }` serial loop with
`futures::future::join_all(configs.map(connect_server))`. Each connection keeps
its existing 15s timeout + graceful-degrade-on-error. Tools register into the
`ToolRegistry` as each server resolves (registry must support concurrent insert
```

Full source: openspec/changes/startup-render-first/design.md

## openspec/changes/startup-render-first/tasks.md

- Source: openspec/changes/startup-render-first/tasks.md
- Lines: 1-77
- SHA256: 7c419f16bca32a35401a1475ac69a8c2a316811de8c725344dc6c44de2f9df1f

```md
# Tasks: Startup Render-First Optimization

> Initial task list (phase: open). Refined during `/comet-build`.
> Convention: `- [x]` done, `- [ ]` pending. One `[in_progress]` at a time.

## Task 1: Restructure `run_repl` startup sequence (render-first)

- [ ] 1.1 Move `EnterAlternateScreen` + `enable_raw_mode` + keyboard flags to the
      very start of `run_repl` (before `start_daemon`).
- [ ] 1.2 Add a minimal splash render: a `Terminal::draw` closure painting
      "Starting wgenty-code…" centered, using `CrosstermBackend` directly (no
      `App`).
- [ ] 1.3 Add an RAII guard that restores terminal state (disable raw mode,
      pop keyboard flags, disable mouse capture, leave alternate screen) on drop
      or early error, complementing the existing panic hook.
- [ ] 1.4 Replace `start_daemon().await` with `start_daemon_bg()` that returns
      `(base_url, ready_rx)`; poll `ready_rx` while refreshing the splash.

## Task 2: Daemon readiness without full init (`start_daemon_bg`)

- [ ] 2.1 Split `DaemonState::new` into `new_shell()` (cheap) + `init_heavy()`
      (MCP + session), so the daemon can `axum::serve` immediately on the shell.
- [ ] 2.2 Spawn `init_heavy()` as a background task; remove the 50×100ms health
      poll (listener is pre-bound, connections succeed at once).
- [ ] 2.3 Verify existing routes still resolve against the shell state before
      heavy init completes (no panics on uninitialized fields).

## Task 3: MCP concurrent connect

- [ ] 3.1 Replace the serial `for config { connect_server().await }` loop in
      `connect_configured_tools` with `join_all` over all auto-start configs.
- [ ] 3.2 Ensure `ToolRegistry` accepts concurrent tool registration (verify
      lock usage); tools register as each server resolves.
- [ ] 3.3 Preserve per-server 15s timeout + graceful-degrade-on-error semantics.

## Task 4: `App::new` skill discovery dedup + non-blocking

- [ ] 4.1 Move `SkillLoader::load_from_dirs` + `ExternalSkillRegistry::discover`
      out of the synchronous `App::new` path into `spawn_blocking` (or fetch via
      daemon `/skills/inventory`).
- [ ] 4.2 Keep `workflow.yaml` + `CommandRouter` setup inside the same blocking
      task; `App` starts with an empty inventory and receives it via event when
      ready.
- [ ] 4.3 Add `AppEvent::SkillsReady` to populate `skill_inventory` /
      `prompt_context` after first render.

## Task 5: First-frame-first in `App::run`

- [ ] 5.1 Move `auto_dream.check_and_run().await` to `tokio::spawn` **after** the
      first `terminal.draw`.
- [ ] 5.2 Move `memory.load()` + `search_memories()` to `tokio::spawn` after the
      first draw.
- [ ] 5.3 Add `AppEvent::MemoriesReady(Vec<String>)`; `startup_memories`
      populated on receipt.
- [ ] 5.4 Ensure first `terminal.draw` executes before any spawned init awaits.

## Task 6: `auto_install` background + skip-if-installed

- [ ] 6.1 Add `is_auto_installed()` fast-path at the top of `run_repl`; skip
      when already installed.
- [ ] 6.2 Otherwise `spawn_blocking(auto_install)` after terminal entry.

## Task 7: Session lazy load

- [ ] 7.1 Replace `session_manager.load_all()` with `load_index()` (IDs +
      metadata only) for the session list.
- [ ] 7.2 Load full message history lazily on `GET /sessions/:id` access.

## Task 8: Verify & validate

- [ ] 8.1 `cargo fmt -- --check`
- [ ] 8.2 `cargo clippy --all-targets -- -D warnings`
- [ ] 8.3 `cargo test --all`
- [ ] 8.4 Manual: `time cargo run --release -- repl` measures reduced
      time-to-first-frame; confirm splash appears before daemon ready.
- [ ] 8.5 Confirm no behavioral regression: MCP tools available before first
      turn, AutoDream still runs, memory recall still injects.
```

