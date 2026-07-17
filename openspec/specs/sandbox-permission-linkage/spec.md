# Spec: sandbox-permission-linkage

## Purpose

Link shell/exec OS sandbox profiles to permission `EffectiveMode` with fail-closed defaults, settings overrides, observability metadata, TUI bypass visibility, and CLI controls.

## Requirements

### Requirement: Mode to sandbox profile matrix

The system SHALL resolve a sandbox `SecurityLevel` and `FailMode` from the call’s `EffectiveMode` for shell/exec tools.

#### Scenario: Plan uses High and HardFail

- **WHEN** `EffectiveMode` is Plan and settings use defaults
- **THEN** resolved level is High and fail mode is HardFail

#### Scenario: Normal uses Standard and HardFail

- **WHEN** `EffectiveMode` is Normal and settings use defaults
- **THEN** resolved level is Standard, fail mode is HardFail, and network policy is Full (package managers; FS remains workspace-scoped)

#### Scenario: Yolo uses Minimal and DegradeWithMark

- **WHEN** `EffectiveMode` is Yolo and settings use defaults
- **THEN** resolved level is Minimal and fail mode is DegradeWithMark

#### Scenario: AcceptEdits shell matches Standard HardFail

- **WHEN** `EffectiveMode` is AcceptEdits for a shell tool
- **THEN** resolved level is Standard and fail mode is HardFail

### Requirement: Settings overrides

The system SHALL load `integrations.sandbox` with `enabled`, optional per-mode level overrides, and optional per-mode fail mode overrides.

#### Scenario: Level override

- **WHEN** `defaults_by_mode.normal` is `minimal`
- **THEN** Normal resolves to Minimal (source SettingsOverride)

#### Scenario: Sandbox disabled

- **WHEN** `enabled` is false
- **THEN** fail mode is DegradeWithMark for all modes and source is Disabled

### Requirement: Fail closed vs marked degrade

Silent direct spawn without metadata SHALL NOT occur on product paths after this change.

#### Scenario: HardFail on infrastructure error

- **WHEN** fail mode is HardFail and sandbox spawn fails
- **THEN** the tool returns an error and MUST NOT run the command via direct spawn

#### Scenario: DegradeWithMark on infrastructure error

- **WHEN** fail mode is DegradeWithMark and sandbox spawn fails
- **THEN** the tool MAY direct-spawn and MUST set metadata `sandbox_bypassed` true

### Requirement: ToolContext carries EffectiveMode

Sandbox resolution SHALL use `ToolContext.effective_mode` only (no process-global mode lock for sandbox).

#### Scenario: Missing mode defaults to Normal

- **WHEN** callers omit or default `effective_mode`
- **THEN** resolution uses Normal (Standard + HardFail defaults)

### Requirement: Observability metadata

Successful or degraded shell results SHOULD include sandbox-related metadata fields: permission mode, level, backend, enforced, bypassed, fail mode, enforcement fidelity (`full`|`partial`|`none`).

#### Scenario: TUI surfaces bypass

- **WHEN** a shell tool result has `sandbox_bypassed=true`
- **THEN** the TUI sets a sticky session indicator and shows a user-visible notice (not metadata-only)

### Requirement: CLI sandbox controls

The system SHALL persist `integrations.sandbox.enabled` via CLI enable/disable and surface resolved mode matrix on status.

#### Scenario: Disable persists

- **WHEN** the user runs `sandbox disable`
- **THEN** settings `enabled` is false and subsequent resolves force DegradeWithMark with bypass marks
