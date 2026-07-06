# subagent-focus-view Specification

## Purpose
TBD - created by archiving change subagent-focus-view. Update Purpose after archive.
## Requirements
### Requirement: Full-screen subagent focus view
The TUI SHALL provide a full-screen focus view that replaces the main chat layout when a user selects a subagent from the status bar and presses Enter. The focus view SHALL display the complete execution timeline of the selected subagent, including all events (Thought, Action, ToolResult, Error, Completion) without truncation.

#### Scenario: Entering focus view from status bar
- **WHEN** the subagent status bar is visible with at least one active subagent
- **AND** the user navigates with ↑↓ to select a subagent and presses Enter
- **THEN** the TUI SHALL replace the main chat + input layout with a full-screen focus view for the selected subagent

#### Scenario: Focus view shows complete event timeline
- **WHEN** a subagent has produced 5 Thought events, 3 Action events, and 2 ToolResult events
- **THEN** the focus view SHALL display all events in chronological order, each with its type icon, elapsed timestamp, and full content (no truncation)

#### Scenario: Focus view real-time updates
- **WHEN** the selected subagent is still Running while the focus view is open
- **THEN** the focus view SHALL continue polling for progress updates and append new events to the timeline as they arrive

#### Scenario: Focus view header shows summary metadata
- **WHEN** the focus view is open for a subagent
- **THEN** the top of the focus view SHALL display the subagent label, status icon, elapsed time, round progress (when available), and cumulative token count

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

### Requirement: Focus view subagent selector bar
The focus view SHALL include a subagent selector bar at the bottom of the screen, listing a "main" entry (for returning to the main chat) followed by all real subagents (active and completed) with their status icons and labels. Placeholder/wrapper nodes that carry no execution information SHALL NOT appear in the selector — this includes both 1:1 wrapper nodes (e.g., a "task:" entry wrapping a single subagent) and 1:N grouping nodes (e.g., a "delegate:" entry that groups several sub-tasks but has no events or messages of its own). Grouping nodes SHALL be excluded from the selector, from active/total counts, and from the active-node list used by the status bar. The selector allows direct switching between subagent views without returning to the main chat. The selector SHALL be the sole keyboard-interactive area in the focus view.

#### Scenario: Selector shows main entry plus real subagents only
- **WHEN** the focus view is open and there are 3 real subagents (1 running, 2 completed)
- **THEN** the selector SHALL display a "main" entry at index 0 followed by exactly those 3 subagents with their status icons and labels
- **AND** the selector SHALL NOT display any placeholder/wrapper node (e.g., a "task:" entry with no events)
- **AND** the selector SHALL be at least 8 rows tall (including borders) so that the "main" entry plus several subagents are visible without scrolling

#### Scenario: Grouping nodes are excluded from selector and counts
- **WHEN** a `delegate` invocation creates a 1:N grouping node (e.g., "delegate: ...") with up to 8 child sub-task nodes, and the grouping node has no events or messages of its own
- **THEN** the selector SHALL NOT display the grouping node
- **AND** the selector SHALL display the child sub-task nodes as real subagents
- **AND** the active count, total count, and active-node list used by the status bar SHALL exclude the grouping node so its stale "Running" status does not inflate counts
- **AND** the grouping node SHALL remain in the tree as a parent for its children, only filtered from display and counts

#### Scenario: Cursor aligns with current subagent on entry
- **WHEN** the user opens the focus view for subagent "explore" from the status bar
- **THEN** the selector cursor (▶) SHALL start on "explore" (the currently viewed subagent)
- **AND** the current-view marker (●) SHALL also be on "explore", so ▶ and ● are aligned on entry

#### Scenario: Selector scrolls to keep cursor visible
- **WHEN** there are more subagents than visible selector rows and the user navigates the cursor past the bottom of the visible window
- **THEN** the selector SHALL scroll so the cursor remains visible
- **AND** the "main" entry MAY scroll out of view when the cursor is far down the list

#### Scenario: Selector wraps around including main
- **WHEN** the user navigates past the last visible subagent in the selector with ↓
- **THEN** the selection SHALL wrap around to the "main" entry (index 0)
- **AND** navigating past the "main" entry with ↑ SHALL wrap to the last visible subagent

#### Scenario: Selector distinguishes cursor from current view
- **WHEN** the focus view is displaying subagent "explore" and the cursor is on a different subagent
- **THEN** the selector SHALL visually distinguish the currently viewed subagent (● marker) from the cursor position (▶ marker)

#### Scenario: Selector border indicates interactive area
- **WHEN** the focus view is open
- **THEN** the selector border SHALL be highlighted (active color) to indicate it is the interactive area
- **AND** the timeline border SHALL be dimmed to indicate it is read-only

#### Scenario: Completed subagents are dimmed
- **WHEN** a subagent in the selector has completed (Completed, Failed, or Cancelled) and is still within the removal window
- **THEN** the selector SHALL render that subagent's label in a dimmed color while keeping its status icon
- **AND** the subagent SHALL remain navigable and switchable until it is removed

#### Scenario: Completed subagents are removed after a delay
- **WHEN** a subagent completed more than N seconds ago (default 10)
- **THEN** the selector SHALL no longer display that subagent
- **AND** the wrap-around length SHALL reflect the reduced visible list
- **AND** the currently viewed subagent SHALL be exempt from removal so its timeline remains accessible

#### Scenario: Removal state persists across focus view sessions
- **WHEN** a subagent completed more than N seconds ago and the user exits and re-enters the focus view
- **THEN** that subagent SHALL still not appear in the selector
- **AND** the completion timestamp SHALL be reset when a new turn starts (subagent tree cleared)

### Requirement: Focus view event type visual distinction
Each event type in the focus view timeline SHALL be visually distinguishable by color and icon, so users can quickly scan the execution flow.

#### Scenario: Thought event display
- **WHEN** a Thought event is displayed in the timeline
- **THEN** it SHALL be rendered with a 💬 icon and a muted color, with the full model text wrapped to the terminal width

#### Scenario: Action event display
- **WHEN** an Action event (tool call) is displayed
- **THEN** it SHALL be rendered with a 🛠 icon and a blue accent color, showing `tool_name("params_summary")`

#### Scenario: ToolResult event display
- **WHEN** a ToolResult event is displayed
- **THEN** it SHALL be rendered with a ✅ or ❌ icon based on success, with a green or red accent color, showing the result summary

#### Scenario: Error event display
- **WHEN** an Error event is displayed
- **THEN** it SHALL be rendered with a ❌ icon and red color, showing the error message and error type

#### Scenario: Completion event display
- **WHEN** a Completion event is displayed
- **THEN** it SHALL be rendered with a ✅ icon and green color, showing the completion status and optional summary

