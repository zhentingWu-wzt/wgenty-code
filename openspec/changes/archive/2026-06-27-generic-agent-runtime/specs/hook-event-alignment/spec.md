# hook-event-alignment Delta Spec

## MODIFIED Requirements

### Requirement: REQ-HEA-001 — new event types

The `HookEvent` enum SHALL be moved from `src/hooks/` to `src/runtime/event.rs` as part of the generic `EventBus`. The existing event types (PreToolUse, PostToolUse, SessionStart, SessionEnd, Notification, Stop, UserPromptSubmit, PermissionRequest) SHALL be preserved. New event types SHALL be added for Runtime-level events.

#### Scenario: Existing hook events preserved after migration

- **WHEN** hooks are configured with any existing event type (PreToolUse, PostToolUse, etc.)
- **AND** the hooks module has been migrated to `src/runtime/`
- **THEN** all existing event types SHALL fire identically to before the migration

#### Scenario: Runtime events added alongside hook events

- **WHEN** the `EventBus` emits a `RuntimeEvent::StateTransition`
- **THEN** the event SHALL be handled by the `EventBus` subscription system
- **AND** existing hook event types SHALL NOT be affected

### Requirement: REQ-HEA-004 — CC hooks format compatibility

`HookManager::from_settings()` SHALL remain compatible with Claude Code hooks format after migration to `src/runtime/`. The `cc_adapter` module SHALL be preserved.

#### Scenario: CC nested array format still parsed after migration

- **WHEN** hooks config uses CC format `{"Stop": [[{"type": "command", "command": "..."}]]}`
- **AND** `HookManager::from_settings()` is called from `src/runtime/`
- **THEN** hooks SHALL be correctly parsed into `Vec<HookDefinition>` with matcher and type fields

### Requirement: REQ-HEA-005 — backward compatibility

All existing hook behavior SHALL be preserved after migration. The `GuardPipeline` (new) SHALL run before `PreToolUse` hooks (existing), matching the current `CometGuard`-before-`PreToolUse` ordering.

#### Scenario: Guard pipeline runs, then PreToolUse hooks

- **WHEN** a tool is about to execute
- **AND** both the `GuardPipeline` and `PreToolUse` hooks are configured
- **THEN** the `GuardPipeline` SHALL evaluate first
- **AND** if the guard allows, `PreToolUse` hooks SHALL then execute
- **AND** this ordering SHALL match the previous comet-guard-before-hooks behavior
