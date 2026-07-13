# Tasks: Startup Render-First Optimization

> Initial task list (phase: open). Refined during `/comet-build`.
> Convention: `- [x]` done, `- [ ]` pending. One `[in_progress]` at a time.

## Task 1: Restructure `run_repl` startup sequence (render-first)

- [x] 1.1 Move `EnterAlternateScreen` + `enable_raw_mode` + keyboard flags to the
      very start of `run_repl` (before `start_daemon`).
- [x] 1.2 Add a minimal splash render: a `Terminal::draw` closure painting
      "Starting wgenty-code…" centered, using `CrosstermBackend` directly (no
      `App`).
- [x] 1.3 Add an RAII guard that restores terminal state (disable raw mode,
      pop keyboard flags, disable mouse capture, leave alternate screen) on drop
      or early error, complementing the existing panic hook.
- [x] 1.4 Replace `start_daemon().await` with `start_daemon_bg()` that returns
      `(base_url, ready_rx)`; poll `ready_rx` while refreshing the splash.

## Task 2: Daemon readiness without full init (`start_daemon_bg`)

- [x] 2.1 Split `DaemonState::new` into `new_shell()` (cheap) + `init_heavy()`
      (MCP + session), so the daemon can `axum::serve` immediately on the shell.
- [x] 2.2 Spawn `init_heavy()` as a background task; remove the 50×100ms health
      poll (listener is pre-bound, connections succeed at once).
- [x] 2.3 Verify existing routes still resolve against the shell state before
      heavy init completes (no panics on uninitialized fields).

## Task 3: MCP concurrent connect

- [x] 3.1 Replace the serial `for config { connect_server().await }` loop in
      `connect_configured_tools` with `join_all` over all auto-start configs.
- [x] 3.2 Ensure `ToolRegistry` accepts concurrent tool registration (verify
      lock usage); tools register as each server resolves.
- [x] 3.3 Preserve per-server 15s timeout + graceful-degrade-on-error semantics.

## Task 4: `App::new` skill discovery dedup + non-blocking

- [x] 4.1 Move `SkillLoader::load_from_dirs` + `ExternalSkillRegistry::discover`
      out of the synchronous `App::new` path into `spawn_blocking` (or fetch via
      daemon `/skills/inventory`).
- [x] 4.2 Keep `workflow.yaml` + `CommandRouter` setup inside the same blocking
      task; `App` starts with an empty inventory and receives it via event when
      ready.
- [x] 4.3 Add `AppEvent::SkillsReady` to populate `skill_inventory` /
      `prompt_context` after first render.

## Task 5: First-frame-first in `App::run`

- [x] 5.1 Move `auto_dream.check_and_run().await` to `tokio::spawn` **after** the
      first `terminal.draw`.
- [x] 5.2 Move `memory.load()` + `search_memories()` to `tokio::spawn` after the
      first draw.
- [x] 5.3 Add `AppEvent::MemoriesReady(Vec<String>)`; `startup_memories`
      populated on receipt.
- [x] 5.4 Ensure first `terminal.draw` executes before any spawned init awaits.

## Task 6: `auto_install` background + skip-if-installed

- [x] 6.1 Add `is_auto_installed()` fast-path at the top of `run_repl`; skip
      when already installed.
- [x] 6.2 Otherwise `spawn_blocking(auto_install)` after terminal entry.

## Task 7: Session lazy load

- [x] 7.1 Replace `session_manager.load_all()` with `load_index()` (IDs +
      metadata only) for the session list.
- [x] 7.2 Load full message history lazily on `GET /sessions/:id` access.

## Task 8: Verify & validate

- [x] 8.1 `cargo fmt -- --check`
- [x] 8.2 `cargo clippy --all-targets -- -D warnings`
- [x] 8.3 `cargo test --all`
- [ ] 8.4 Manual: `time cargo run --release -- repl` measures reduced
      time-to-first-frame; confirm splash appears before daemon ready.
- [x] 8.5 Confirm no behavioral regression: MCP tools available before first
      turn, AutoDream still runs, memory recall still injects.
