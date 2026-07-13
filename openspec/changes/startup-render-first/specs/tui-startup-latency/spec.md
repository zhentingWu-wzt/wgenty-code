# tui-startup-latency Specification

## Purpose

Governs the latency from process launch to the first rendered TUI frame and the
non-blocking initialization of heavy startup work (daemon, MCP, skills, AutoDream,
memory). Ensures the user sees a frame within terminal-switch time while heavy
init completes concurrently in the background.

## ADDED Requirements

### Requirement: First TUI frame renders before daemon fully initializes

The REPL SHALL enter the alternate screen and render a splash frame before
`start_daemon` completes its heavy initialization (MCP connect, session load),
so the user perceives the application within terminal-switch time.

#### Scenario: Launch shows splash before daemon ready

- **WHEN** the user runs `wgenty-code repl`
- **THEN** the terminal SHALL enter the alternate screen and render a splash
  frame before MCP servers connect and before `DaemonState` heavy init completes

#### Scenario: Splash refreshes while daemon starts

- **WHEN** the daemon is starting in the background
- **THEN** the splash SHALL remain visible and the terminal SHALL not block on
  daemon readiness before transitioning to the main `App`

### Requirement: Daemon becomes ready on TCP bind without waiting for heavy init

`start_daemon` SHALL return a usable base URL once the TCP listener is bound and
`axum::serve` is spawned, without waiting for MCP connections or full session
loading to complete.

#### Scenario: Daemon accepts connections immediately after bind

- **WHEN** `start_daemon_bg` binds the listener and spawns the server
- **THEN** it SHALL return `(base_url, ready_rx)` without polling a health
  endpoint up to 50 times

#### Scenario: Heavy init runs in background

- **WHEN** the daemon is serving with its shell state
- **THEN** MCP connections and session loading SHALL proceed as background tasks
  and SHALL NOT block the daemon from accepting requests

### Requirement: MCP servers connect concurrently

Multiple configured MCP servers SHALL connect in parallel rather than serially,
so total connect time approaches the maximum individual connect time instead of
the sum.

#### Scenario: Multiple MCP servers connect in parallel

- **WHEN** two or more MCP servers are configured with `auto_start`
- **THEN** their connections SHALL be initiated concurrently via `join_all`
  (or equivalent), not sequentially

#### Scenario: Per-server timeout and graceful degradation preserved

- **WHEN** an MCP server fails to connect within its timeout
- **THEN** the daemon SHALL continue without that server's tools, preserving the
  existing 15s timeout and graceful-degrade semantics

### Requirement: Skill inventory populates asynchronously without blocking first frame

`App::new` SHALL NOT perform synchronous skill discovery on the startup critical
path. The skill inventory SHALL be loaded via `spawn_blocking` and delivered
through an event after the first frame renders.

#### Scenario: App starts without blocking on skill discovery

- **WHEN** `App::new` is constructed
- **THEN** `SkillLoader::load_from_dirs` and `ExternalSkillRegistry::discover`
  SHALL run in a `spawn_blocking` task, not inline

#### Scenario: Skill inventory arrives via event

- **WHEN** the background skill discovery completes
- **THEN** an `AppEvent::SkillsReady` SHALL populate the prompt's skill inventory,
  and the first user turn SHALL include the inventory in practice (discovery
  completes faster than human typing)

### Requirement: AutoDream consolidation runs after the first frame

`App::run` SHALL render the first frame before invoking AutoDream
`check_and_run`, so LLM-backed consolidation never blocks the first visible frame.

#### Scenario: First draw precedes AutoDream

- **WHEN** `App::run` begins its main loop
- **THEN** the first `terminal.draw` SHALL execute before
  `auto_dream.check_and_run().await` is spawned

#### Scenario: AutoDream runs in background

- **WHEN** AutoDream's gate passes at startup
- **THEN** consolidation SHALL run via `tokio::spawn` and its result SHALL be
  fire-and-forget, never blocking the render loop

### Requirement: Memory recall runs after the first frame and injects via event

`App::run` SHALL render the first frame before loading and recalling memories.
Recalled memories SHALL be delivered via an event so they appear without blocking
startup.

#### Scenario: First draw precedes memory recall

- **WHEN** `App::run` begins its main loop
- **THEN** the first `terminal.draw` SHALL execute before `memory.load()` and
  `search_memories()`

#### Scenario: Recalled memories arrive via event

- **WHEN** background memory recall completes
- **THEN** an `AppEvent::MemoriesReady` SHALL populate `startup_memories`, which
  SHALL be injected into the conversation without a behavioral regression

### Requirement: Bundled skills auto-install skips when already installed

`run_repl` SHALL skip `auto_install` when bundled skills are already present, and
SHALL run any required install in the background after terminal entry.

#### Scenario: Skip install when already present

- **WHEN** `is_auto_installed()` returns true at startup
- **THEN** `auto_install` SHALL NOT perform any disk write

#### Scenario: Background install when not present

- **WHEN** bundled skills are not yet installed
- **THEN** the install SHALL run via `spawn_blocking` after the terminal enters
  the alternate screen, not before

### Requirement: Session list loads metadata index only at startup

The session manager SHALL load only a metadata index (IDs + summary + mtime) at
startup for the session list, loading full message history lazily on access.

#### Scenario: Startup loads metadata index only

- **WHEN** the daemon starts and recovers persisted sessions
- **THEN** it SHALL load session IDs and metadata without reading every session's
  full message history

#### Scenario: Full history loaded on access

- **WHEN** a client requests a specific session's detail
- **THEN** the full message history SHALL be loaded on demand
