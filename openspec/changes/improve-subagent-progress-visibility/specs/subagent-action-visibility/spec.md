## ADDED Requirements

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
Each subagent node SHALL maintain a short history of recent tool calls (name + params), visible in the overlay panel, so users can trace what actions the subagent has taken.

#### Scenario: Subagent has made multiple tool calls
- **WHEN** a subagent has called `grep`, `file_read`, and `file_read` in sequence
- **THEN** the overlay panel SHALL display up to the 3 most recent tool calls beneath that node, newest first

#### Scenario: Action history is bounded
- **WHEN** a subagent has made more than 10 tool calls
- **THEN** the action log SHALL retain only the 10 most recent entries, dropping the oldest

#### Scenario: Completed subagent action history
- **WHEN** a subagent reaches Completed status
- **THEN** the action log SHALL remain visible in the panel (not cleared) so users can review what the subagent did

### Requirement: Model text is displayed alongside tool calls
The TUI SHALL display the model's text responses alongside tool calls so users can see the think→call→think→call loop. The text snapshot shows what the model is analyzing or concluding; the action log shows what tools it called.

#### Scenario: Model text followed by tool call
- **WHEN** a subagent outputs text "I need to find where authentication logic is defined" then calls `grep("fn authenticate")`
- **THEN** the TUI SHALL display the text snapshot above the current tool action, so the display reads: the model's thought → then the action it took

#### Scenario: Tool call followed by model analysis
- **WHEN** a subagent completes a `file_read` call and the model responds with "Found the auth module, it needs refactoring in 3 places"
- **THEN** the TUI SHALL update the text snapshot to show the model's analysis, with the completed tool call now in the action history

### Requirement: Inline subagent card shows current action with context
The inline subagent card rendered in the chat area SHALL show the current tool call with parameters and the most recent model text, so users can see what the subagent is doing without opening the overlay panel.

#### Scenario: Inline card during active subagent
- **WHEN** a subagent is Running with text snapshot "Analyzing the auth module structure…" and current tool `file_read("src/auth.rs")`
- **THEN** the inline card SHALL display the tool call with params and a dimmed preview of the model's text

#### Scenario: Inline card when subagent has no text yet
- **WHEN** a subagent is Running but has no text snapshot yet (first round, still streaming)
- **THEN** the inline card SHALL display "thinking…" and no text preview
