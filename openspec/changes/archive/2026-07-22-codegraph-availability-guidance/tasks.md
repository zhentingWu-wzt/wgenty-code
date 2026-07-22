# Implementation Tasks

> Source plan: `plan.md`. All tasks verified implemented against `src/mcp/codegraph.rs` and wiring sites.

## Phase 1: Foundation

- [x] 1. Config schema for per-project dismissal — `CodegraphSettings { dismissed_paths }` in `src/config/services.rs`, `#[serde(default)]` on `IntegrationsConfig.codegraph`
- [x] 2. `CodegraphInstallState` probe module — `src/mcp/codegraph.rs`: `probe_install_state()` + `classify_install_state()` (pure, unit-testable), 4-state enum

## Phase 2: Wiring

- [x] 3. Short-circuit `NotInstalled`/`Dismissed` in `connect_configured_tools()` — skip codegraph spawn; `NotInitialized` still connects
- [x] 4. `PromptContext` carries `CodegraphInstallState` + environment-layer injection of `CodeGraph status: <state>`
- [x] 5. Set `codegraph_state` at `PromptContext` construction sites (REPL / reload / query)
- [x] 6. Update `base.md` codegraph guidance text — replace generic fallback line with reference to injected state + ask_user_question behavior

## Phase 3: Dismissal tool

- [x] 7. `dismiss_codegraph_guidance` meta tool — `is_read_only() = false`, canonicalize + dedupe append to `dismissed_paths`, save settings.json, ToolRegistry registration

## Phase 4: CLI startup notice

- [x] 8. Print availability notice at REPL + query startup — stderr notice for `NotInstalled`/`NotInitialized`/`ConnectionError`; silent for `Connected`/`Dismissed`; daemon mode `tracing::warn!`

## Phase 5: TUI status bar

- [x] 9. Upgrade TUI CG indicator to `CodegraphInstallState` — `detect_codegraph_status()` real probe; `codegraph_status_span()` ⚠/✗/○ mapping; refresh-point type sync

## Phase 6: Verification

- [x] 10. Lint, format, tests, changelog — `probe_install_state` classification tests, dismiss-tool dedupe, notice-silent-under-dismissed
