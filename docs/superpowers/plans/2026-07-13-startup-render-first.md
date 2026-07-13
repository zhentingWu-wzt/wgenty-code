---
change: startup-render-first
design-doc: openspec/changes/startup-render-first/design.md
base-ref: 90ef1961fdba50a02325a19df0ac61782c6e5c8f
---

# Implementation Plan: Startup Render-First Optimization

Source: `openspec/changes/startup-render-first/design.md` + `tasks.md`.
Goal: first TUI frame renders within terminal-switch time; heavy init runs
concurrently in the background.

## Execution order (tasks are interdependent; sequential)

### Task 1 — Restructure `run_repl` (render-first)  [`src/cli/args.rs`]
1. Move `EnterAlternateScreen` / `enable_raw_mode` / keyboard flags / mouse
   capture to the START of `run_repl`, before `start_daemon`.
2. Add a minimal splash draw using `CrosstermBackend` + `Terminal::draw`
   ("Starting wgenty-code…"). No `App` yet.
3. Add an RAII guard struct that restores terminal state on drop/early error
   (disable raw mode, pop flags, disable mouse, leave alt screen); complements
   the existing panic hook.
4. Replace `start_daemon().await` with `start_daemon_bg()` returning
   `(base_url, ready_rx)`; poll `ready_rx` while refreshing splash.

### Task 2 — Daemon readiness without full init  [`src/tui/util.rs`, `src/daemon/state.rs`]
1. Split `DaemonState::new` into `new_shell()` (cheap: coordinator, policy,
   registries, mcp_manager) + `init_heavy()` (MCP connect, session index).
2. `start_daemon_bg`: bind listener, build shell state, spawn `axum::serve`,
   spawn `init_heavy()` as background task, return `(base_url, ready_rx)`
   immediately. Remove the 50×100ms health poll.
3. Verify routes resolve against shell state before heavy init (no panics on
   uninitialized fields; use `Option`/empty defaults).

### Task 3 — MCP concurrent connect  [`src/mcp/mod.rs`, `src/daemon/state.rs`]
1. Replace serial `for config { connect_server().await }` with
   `futures::future::join_all(configs.map(|c| connect_server(c)))`.
2. Verify `ToolRegistry` concurrent insert (lock-guarded); tools register as
   each server resolves.
3. Preserve per-server 15s timeout + graceful-degrade.

### Task 4 — `App::new` skill discovery non-blocking  [`src/tui/app/mod.rs`]
1. Move `SkillLoader::load_from_dirs` + `ExternalSkillRegistry::discover` +
   `workflow.yaml`/`CommandRouter` setup into `tokio::task::spawn_blocking`.
2. `App` starts with empty skill inventory.
3. Add `AppEvent::SkillsReady(Vec<SkillEntry>)`; on receipt, update
   `prompt_context` skills + `command_router`/`completion_engine` entries.

### Task 5 — First-frame-first in `App::run`  [`src/tui/app/mod.rs`]
1. Move `auto_dream.check_and_run().await` to `tokio::spawn` AFTER first
   `terminal.draw`.
2. Move `memory.load()` + `search_memories()` to `tokio::spawn` after first draw.
3. Add `AppEvent::MemoriesReady(Vec<String>)`; populate `startup_memories` on
   receipt.
4. Ensure first `terminal.draw` runs before any spawned init awaits.

### Task 6 — `auto_install` background + skip  [`src/cli/args.rs`, `src/knowledge/embedded.rs`]
1. `if is_auto_installed() { skip }` fast-path at top of `run_repl`.
2. Else `tokio::task::spawn_blocking(auto_install)` after terminal entry.

### Task 7 — Session lazy load  [`src/daemon/state.rs` + session_manager]
1. Replace `session_manager.load_all()` with `load_index()` (IDs + summary +
   mtime) for the session list.
2. Load full message history lazily on `GET /sessions/:id` access.

### Task 8 — Verify & validate
1. `cargo fmt -- --check`
2. `cargo clippy --all-targets -- -D warnings`
3. `cargo test --all`
4. Manual: `time cargo run --release -- repl` — confirm splash before daemon
   ready; MCP tools available before first turn; AutoDream still runs; memory
   recall still injects.

## Notes
- Tasks 1+2 are foundational (the startup-sequence restructure); do them first.
- Tasks 3-7 are independent enhancements layered on the new sequence.
- Task 4 and Task 5 both add `AppEvent` variants — implement together to avoid
  merge friction.
- Non-testable timing scenarios (splash-before-daemon) verified manually in
  Task 8; testable behavior (event handling, skip-if-installed) gets unit tests.
