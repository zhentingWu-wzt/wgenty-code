## ADDED Requirements

### Requirement: CodeGraph install-state probe
The system SHALL probe CodeGraph availability via a synchronous, pure function of `&Settings` and the working directory, classifying into exactly four states: `Ready` (binary on PATH and `.codegraph/` index dir present), `NotInstalled` (binary not on PATH), `NotInitialized` (binary present but no `.codegraph/` index dir), and `Dismissed` (working dir canonicalized into `dismissed_paths`). `Dismissed` SHALL take precedence over all other states. The probe SHALL NOT depend on the async MCP handshake result.

#### Scenario: Not installed
- **WHEN** `which("codegraph")` fails
- **THEN** the probe SHALL return `NotInstalled` regardless of `.codegraph/` presence

#### Scenario: Installed but not initialized
- **WHEN** `which("codegraph")` succeeds but `<working_dir>/.codegraph/` does not exist
- **THEN** the probe SHALL return `NotInitialized`

#### Scenario: Dismissed takes precedence
- **WHEN** the canonicalized working dir is in `dismissed_paths`
- **AND** codegraph is not installed
- **THEN** the probe SHALL return `Dismissed`

#### Scenario: Ready
- **WHEN** codegraph is on PATH and `<working_dir>/.codegraph/` exists
- **THEN** the probe SHALL return `Ready`

### Requirement: NotInstalled/Dismissed short-circuit
In `connect_configured_tools()`, the system SHALL short-circuit CodeGraph MCP injection: `NotInstalled` and `Dismissed` SHALL remove codegraph from the auto-start configs and skip spawn entirely. `NotInitialized` SHALL still connect (the `serve --mcp` process starts successfully and the agent may self-initialize).

#### Scenario: NotInstalled skips spawn
- **WHEN** the probe returns `NotInstalled`
- **THEN** codegraph SHALL NOT be spawned and no spawn-failure noise SHALL be produced

#### Scenario: NotInitialized still connects
- **WHEN** the probe returns `NotInitialized`
- **THEN** codegraph SHALL remain in the auto-start configs and be connected normally

### Requirement: Per-project dismissal persistence
The system SHALL persist dismissal per working directory in `integrations.codegraph.dismissed_paths` (`Vec<PathBuf>`, default empty). The working dir SHALL be canonicalized before comparison and storage, with deduplication. `#[serde(default)]` SHALL ensure legacy `settings.json` files without the field load without error.

#### Scenario: Legacy config compatibility
- **WHEN** a `settings.json` predating this feature is loaded
- **THEN** `dismissed_paths` SHALL default to an empty vec and load without error

#### Scenario: Dedup on dismiss
- **WHEN** the same working dir is dismissed twice
- **THEN** `dismissed_paths` SHALL contain the canonicalized path exactly once

### Requirement: CLI startup availability notice
The system SHALL print a single-line availability notice to stderr at REPL and `query` startup for `NotInstalled`, `NotInitialized`, and `ConnectionError` states, each stating the degradation to grep/lsp fallback and the remediation command. `Connected` and `Dismissed` SHALL be silent. In daemon mode the notice SHALL be emitted via `tracing::warn!` instead of stderr. No notice SHALL be interactive (no y/n prompt at startup).

#### Scenario: NotInstalled notice
- **WHEN** startup probe returns `NotInstalled`
- **THEN** a stderr line SHALL name the missing binary and suggest `npm i -g @colbymchenry/codegraph`

#### Scenario: Silent when connected
- **WHEN** startup probe returns `Ready`/`Connected`
- **THEN** no notice SHALL be printed

#### Scenario: Silent when dismissed
- **WHEN** startup probe returns `Dismissed`
- **THEN** no notice SHALL be printed

### Requirement: Prompt injection of availability state
The system SHALL inject a concrete `CodeGraph status: <state>` line into the per-turn environment context, replacing the prior generic fallback instruction. The agent SHALL, when about to perform code navigation while state is `NotInstalled`/`NotInitialized` and not `Dismissed`, first use `ask_user_question` offering: install/initialize now, don't remind again, or skip this time. `Connected` SHALL use codegraph normally; `Dismissed` SHALL fall back to grep/lsp without asking.

#### Scenario: State injected to agent
- **WHEN** a turn is assembled and codegraph is `NotInitialized`
- **THEN** the environment context SHALL contain a `CodeGraph status: not_initialized` line with remediation hint

#### Scenario: Dismissed falls back silently
- **WHEN** state is `Dismissed`
- **THEN** the agent SHALL fall back to grep/lsp without issuing an `ask_user_question`

### Requirement: dismiss_codegraph_guidance meta tool
The system SHALL provide a `dismiss_codegraph_guidance` tool (meta class) with `is_read_only()` returning `false`. It SHALL canonicalize the current working dir (falling back to the raw path on canonicalize failure), dedupe-append it to `dismissed_paths`, persist `settings.json`, and return a confirmation. An optional `path` input SHALL default to the current working dir.

#### Scenario: Persist dismissal
- **WHEN** the agent calls `dismiss_codegraph_guidance`
- **THEN** the current working dir SHALL be canonicalized, deduped into `dismissed_paths`, and `settings.json` SHALL be saved

#### Scenario: Non-read-only declaration
- **WHEN** the tool registry queries `is_read_only()`
- **THEN** it SHALL return `false` so guardian treats it as a state-mutating tool

### Requirement: TUI status bar indicator upgrade
The TUI CodeGraph status indicator SHALL reflect the real probed `CodegraphInstallState` rather than the configured `status` field. The indicator SHALL map `Ready`/`Connected` to a success glyph, `NotInstalled`/`NotInitialized` to a warning glyph, `ConnectionError` to an error glyph, and `Dismissed` to a dim glyph.

#### Scenario: Warning glyph for not initialized
- **WHEN** the probed state is `NotInitialized`
- **THEN** the status bar SHALL render a warning-colored glyph

#### Scenario: Dim glyph for dismissed
- **WHEN** the probed state is `Dismissed`
- **THEN** the status bar SHALL render a dim glyph
