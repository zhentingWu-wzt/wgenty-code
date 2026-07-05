# subagent-focus-view Specification

## ADDED Requirements

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
The focus view SHALL support keyboard navigation for scrolling the event timeline and shall return to the main chat layout when the user presses Esc. The focus view SHALL also provide a subagent selector bar at the bottom for direct switching between subagent focus views without returning to the main chat.

#### Scenario: Scrolling the event timeline
- **WHEN** the focus view is open and the event timeline exceeds the visible area
- **AND** the focus is on the timeline area (default)
- **THEN** ↑↓ keys SHALL scroll the timeline one line at a time, and PageUp/PageDown SHALL scroll by 10 lines

#### Scenario: Tab toggles between timeline and subagent selector
- **WHEN** the user presses Tab while in the focus view
- **THEN** the focus SHALL toggle between the event timeline area and the subagent selector bar
- **AND** the currently focused area SHALL be visually indicated (e.g., highlight border)

#### Scenario: Switching to another subagent from within focus view
- **WHEN** the user presses Tab to focus the subagent selector bar
- **AND** navigates with ↑↓ to select another subagent and presses Enter
- **THEN** the focus view SHALL switch to display the selected subagent's event timeline
- **AND** the timeline scroll position SHALL reset to the latest event

#### Scenario: Exiting focus view returns to main chat
- **WHEN** the user presses Esc while in the focus view
- **THEN** the TUI SHALL close the focus view and restore the main chat + input layout
- **AND** the input box SHALL regain focus for text entry
- **AND** the subagent status bar SHALL remain visible if subagents are still running

### Requirement: Focus view subagent selector bar
The focus view SHALL include a subagent selector bar at the bottom of the screen, listing all subagents (active and completed) with their status icons and labels. The selector bar allows direct switching between subagent views without returning to the main chat.

#### Scenario: Selector bar shows all subagents
- **WHEN** the focus view is open and there are 3 subagents (1 running, 2 completed)
- **THEN** the selector bar SHALL display all 3 subagents with their status icons and labels, with the currently viewed subagent highlighted

#### Scenario: Selector bar wraps around
- **WHEN** the user navigates past the last subagent in the selector bar with ↓
- **THEN** the selection SHALL wrap around to the first subagent
- **AND** navigating past the first subagent with ↑ SHALL wrap to the last

#### Scenario: Selector bar indicates current view
- **WHEN** the focus view is displaying subagent "explore"
- **THEN** the selector bar SHALL visually distinguish "explore" as the currently viewed subagent (e.g., reverse video or arrow marker)

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
