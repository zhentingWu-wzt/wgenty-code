# Brainstorm Summary - startup-render-first

**Date:** 2026-07-13
**Phase:** design
**Approach confirmed:** A (render-first, full)
**Skill-dedup decision:** Option 1 (spawn_blocking + `AppEvent::SkillsReady`)

## Problem (crystallized)

All blocking init runs serially before the first `terminal.draw`
(`src/tui/app/mod.rs:632`):

1. `auto_install()` sync disk write (`cli/args.rs:118`)
2. `start_daemon().await` blocks terminal entry (`cli/args.rs:140`): serial MCP
   connect (each ≤15s) + `session_manager.load_all()` + 50×100ms health poll
3. `App::new()` duplicates skill discovery + `ExternalSkillRegistry::discover`
   (`app/mod.rs:202`)
4. `PluginManager.load_all().await` (`args.rs:187`)
5. `App::run()` pre-render: AutoDream LLM consolidation (default-on, gate passes
   by default) + memory load/recall (`app/mod.rs:548-610`)

AutoDream `enabled: true` + `ConsolidationState::default()` =>
`last_consolidated_at = now - (min_hours+1)` => time gate passes every launch.

## Confirmed design

Render-first: enter terminal -> splash -> daemon bg (bind-fast, heavy init
deferred) -> poll readiness w/ splash refresh -> `App` -> first draw immediate ->
spawn AutoDream + memory recall -> inject via events.

Seven component changes (see `design.md`). Key decisions:
- **MCP**: `join_all` concurrent connect, per-server 15s timeout preserved.
- **Skill dedup**: `spawn_blocking` + `AppEvent::SkillsReady` (no new API).
- **AutoDream/memory**: `tokio::spawn` after first draw; `AppEvent::MemoriesReady`.
- **auto_install**: `is_auto_installed()` fast-path + `spawn_blocking`.
- **Sessions**: `load_index()` (metadata only); lazy full load on access.

## Rejected alternatives

- **Approach B (defer heavy init, no terminal restructure)**: lower risk but
  leaves a pre-terminal gap; user chose A for max impact.
- **Skill dedup Option 2 (daemon `/skills/inventory` API)**: adds startup HTTP
  dependency + new endpoint; rejected for focus.
- **Skill dedup Option 3 (measure-first)**: doesn't remove the blocking path.

## Risks

- `run_repl` restructure: splash must restore terminal on early error (RAII guard
  + existing panic hook).
- MCP concurrent insert into `ToolRegistry`: verify lock usage.
- Deferred AutoDream/memory: results arrive one frame later (event-injected, no
  behavior regression).
