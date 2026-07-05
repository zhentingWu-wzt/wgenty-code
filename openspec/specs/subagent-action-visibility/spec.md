# subagent-action-visibility Specification

## Purpose
TBD - created by archiving change improve-subagent-progress-visibility. Update Purpose after archive.
## Requirements
### Requirement: Subagent tool calls are visible with key parameters
The TUI SHALL display the tool calls made by each subagent, including the tool name and key parameters, so users can perceive what actions the model is taking.

#### Scenario: Subagent calls a tool with parameters
- **WHEN** a subagent calls `file_read` with `file_path = "src/auth.rs"`
- **THEN** the TUI SHALL display `file_read("src/auth.rs")` as the current action for that subagent node

#### Scenario: Subagent calls a tool with multiple parameters
- **WHEN** a subagent calls `grep` with `pattern = "fn authenticate"` and `path = "src/"`
- **THEN** the TUI SHALL display `grep("fn authenticate", src/)` extracting the most meaningful 1-2 params

#### Scenario: Subagent calls a tool with long parameters
- **WHEN** a tool parameter value exceeds 80 characters
- **THEN** the params display SHALL be truncated to ~80 chars with "…" suffix

#### Scenario: Subagent has not called any tool yet
- **WHEN** a subagent is Running but has not yet made its first tool call (still in initial API call)
- **THEN** the TUI SHALL display "thinking…" as the current action

### Requirement: Subagent action history shows recent tool calls
Each subagent node SHALL maintain a complete, unbounded history of tool calls (name + params), visible in the overlay panel and detail view. The action log SHALL NOT be truncated in the transcript; the TUI panel MAY truncate the display for readability.

#### Scenario: Subagent has made multiple tool calls
- **WHEN** a subagent has called `grep`, `file_read`, and `file_read` in sequence
- **THEN** the overlay panel SHALL display all tool calls beneath that node, with the ability to scroll when the list exceeds the visible area

#### Scenario: Action history is persisted, not truncated
- **WHEN** a subagent has made more than 50 tool calls
- **THEN** all tool calls SHALL be preserved in the SQLite transcript; the TUI panel SHALL support scrolling/paging to view beyond the visible window

#### Scenario: Completed subagent action history
- **WHEN** a subagent reaches Completed status
- **THEN** the complete action log SHALL be written to SQLite and remain viewable via the detail view

### Requirement: Model text is displayed alongside tool calls
The TUI SHALL display the model's text responses alongside tool calls so users can see the think→call→think→call loop. The text snapshot shows what the model is analyzing or concluding; the action log shows what tools it called.

#### Scenario: Model text followed by tool call
- **WHEN** a subagent outputs text "I need to find where authentication logic is defined" then calls `grep("fn authenticate")`
- **THEN** the TUI SHALL display the text snapshot above the current tool action, so the display reads: the model's thought → then the action it took

#### Scenario: Tool call followed by model analysis
- **WHEN** a subagent completes a `file_read` call and the model responds with "Found the auth module, it needs refactoring in 3 places"
- **THEN** the TUI SHALL update the text snapshot to show the model's analysis, with the completed tool call now in the action history

#### Scenario: Full text preserved in transcript
- **WHEN** a subagent produces a text response of any length
- **THEN** the full text SHALL be recorded in the SQLite transcript; the TUI text snapshot MAY truncate for inline display but the detail view SHALL show the complete text

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

