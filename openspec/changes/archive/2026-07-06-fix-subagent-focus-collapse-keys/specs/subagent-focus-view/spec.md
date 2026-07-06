## MODIFIED Requirements

### Requirement: Focus view navigation and exit
The focus view SHALL use the subagent selector as the sole keyboard-interactive area for navigation: ↑↓ SHALL navigate the selector (the "main" entry plus all visible subagents) and Enter SHALL switch the displayed subagent or exit to the main chat. The event timeline SHALL be read-only, scrollable only via mouse wheel. Fold shortcuts (`t`, `Ctrl+O`, `Ctrl+E`) SHALL be always available and SHALL NOT depend on any focus area; while the focus view is open they SHALL operate on the focus view's timeline and SHALL NOT affect the main chat's message collapse state. The focus view SHALL return to the main chat layout when the user presses Esc or selects the "main" entry and presses Enter.

#### Scenario: Arrow keys navigate the selector
- **WHEN** the focus view is open
- **THEN** ↑↓ SHALL move the selector cursor (▶) among the "main" entry and all visible subagents, wrapping at both ends
- **AND** ↑↓ SHALL NOT scroll the event timeline

#### Scenario: Enter switches subagent or exits to main
- **WHEN** the selector cursor is on a subagent and the user presses Enter
- **THEN** the focus view SHALL switch to display that subagent's event timeline
- **AND** the timeline scroll position SHALL reset to the latest event
- **WHEN** the selector cursor is on the "main" entry and the user presses Enter
- **THEN** the TUI SHALL close the focus view and restore the main chat layout

#### Scenario: Timeline scrolls only via mouse wheel
- **WHEN** the focus view is open and the event timeline exceeds the visible area
- **THEN** mouse wheel SHALL scroll the timeline (ScrollUp toward older, ScrollDown toward newer)
- **AND** PageUp/PageDown SHALL have no effect inside the focus view

#### Scenario: Fold toggle is always available
- **WHEN** the user presses `t` inside the focus view
- **THEN** the focus view SHALL toggle fold/expand of all tool calls in the timeline
- **AND** this behavior SHALL NOT depend on any focus area

#### Scenario: Ctrl+E toggles fold of all tool calls in focus view
- **WHEN** the focus view is open and the user presses Ctrl+E
- **THEN** the focus view SHALL toggle fold/expand of all tool calls in the timeline (same as `t`)
- **AND** SHALL NOT affect the main chat's message collapse state

#### Scenario: Ctrl+O toggles fold of the last tool call in focus view
- **WHEN** the focus view is open and the user presses Ctrl+O
- **THEN** the focus view SHALL toggle fold/expand of the last tool call in the timeline
- **AND** SHALL NOT affect the main chat's message collapse state

#### Scenario: Tab is a no-op
- **WHEN** the user presses Tab inside the focus view
- **THEN** no focus toggle SHALL occur; Tab SHALL be a no-op

#### Scenario: Exiting focus view returns to main chat
- **WHEN** the user presses Esc while in the focus view
- **THEN** the TUI SHALL close the focus view and restore the main chat + input layout
- **AND** the input box SHALL regain focus for text entry
- **AND** the subagent status bar SHALL remain visible if subagents are still running
