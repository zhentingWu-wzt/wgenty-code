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
- it already uses the reserved-names set; guard with the existing lock).

### 4. `App::new` skill discovery dedup + non-blocking (`src/tui/app/mod.rs`) — **confirmed: Option 1 (spawn_blocking + event)**
- `SkillLoader::load_from_dirs` + `ExternalSkillRegistry::discover` move out of
  the synchronous `App::new` critical path into a `tokio::task::spawn_blocking`.
- `workflow.yaml` read + `CommandRouter` setup run inside the same blocking task.
- `App` starts with an empty skill inventory; the populated inventory arrives via
  a new `AppEvent::SkillsReady(Vec<SkillEntry>)`, which updates `prompt_context`.
- Rationale: skill discovery completes in tens of ms, far faster than the user's
  first typed message, so the first turn's system prompt will in practice always
  include the inventory. No new daemon API needed (keeps the change focused).

### 5. `App::run` first-frame-first (`src/tui/app/mod.rs`)
- Move the AutoDream `check_and_run().await` and the memory load+recall blocks
  to **after** the first `terminal.draw`, wrapped in `tokio::spawn`.
- Recalled memories injected via a new `AppEvent::MemoriesReady(Vec<String>)`;
  `startup_memories` populated on event receipt (one frame later).
- AutoDream runs fully in background; its result is fire-and-forget (already
  best-effort).

### 6. `auto_install` background + skip (`src/cli/args.rs`, `src/knowledge/embedded.rs`)
- Fast-path: `if is_auto_installed() { skip }` before any write.
- Otherwise `tokio::task::spawn_blocking(auto_install)` after terminal entry.

### 7. Session lazy load (`src/daemon/state.rs` / session_manager)
- `load_all()` -> `load_index()`: read only session IDs + metadata (mtime,
  summary) for the session list. Full message history loaded on
  `GET /sessions/:id` access.

## Concurrency / safety

- `ToolRegistry` concurrent tool insert during background MCP connect: the
  daemon's registry is behind an `Arc<RwLock>`; inserts acquire write lock
  briefly. First user turn cannot submit until `App::run` enters its loop, by
  which time the registry is populated or the turn blocks on tool resolution
  (acceptable - same as today's serial behavior, just non-blocking for render).
- Splash cleanup on early error: a guard struct (RAII) restores terminal state
  on drop, complementing the panic hook.
- AutoDream lock (`is_consolidating`) already serializes across processes;
  moving to background does not change correctness.

## Out of scope (non-goals)

- No daemon API contract changes beyond optionally adding a lightweight
  `GET /skills/inventory` (read-only) - the design phase will decide fetch-vs-
  share for the dedup.
- No TUI feature/behavior changes.
- No settings/config additions.
