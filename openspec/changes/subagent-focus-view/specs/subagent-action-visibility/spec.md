# subagent-action-visibility Specification

## MODIFIED Requirements

### Requirement: Inline subagent card shows current action with context
The inline subagent card SHALL NOT be rendered in the main chat area. Instead, the current tool call with parameters and the most recent model text SHALL be displayed in the subagent status bar (below the input area) and the full execution timeline SHALL be available in the focus view.

#### Scenario: Chat area remains clean during subagent execution
- **WHEN** a subagent is Running with text snapshot "Analyzing the auth module structure…" and current tool `file_read("src/auth.rs")`
- **THEN** the main chat area SHALL NOT display any inline subagent card or tree structure
- **AND** the subagent status bar SHALL display the current tool and a compact label for the subagent

#### Scenario: No inline card when subagent has no text yet
- **WHEN** a subagent is Running but has no text snapshot yet (first round, still streaming)
- **THEN** the main chat area SHALL NOT display any inline subagent card
- **AND** the subagent status bar SHALL display the subagent label with a "thinking…" indicator

## REMOVED Requirements
<!-- The "Inline subagent card shows current action with context" requirement is modified, not removed. The scenarios above replace the old inline card scenarios. -->
