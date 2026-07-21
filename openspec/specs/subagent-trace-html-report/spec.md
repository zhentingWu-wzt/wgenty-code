# subagent-trace-html-report Specification

## Purpose
TBD - created by archiving change subagent-visualization. Update Purpose after archive.
## Requirements
### Requirement: HTML report is self-contained
The system SHALL generate a single HTML file with all CSS and JavaScript inlined, requiring no external network dependencies or CDN resources.

#### Scenario: Offline viewing
- **WHEN** the HTML report is opened in a browser without network access
- **THEN** all styling, interactivity, and data SHALL render correctly

### Requirement: Collapsible call tree
The HTML report SHALL render the subagent call tree with expand/collapse functionality, showing each node's status icon, label, duration, token usage, and round count.

#### Scenario: Default collapsed view
- **WHEN** the HTML report is first opened
- **THEN** root-level nodes SHALL be visible and child nodes beyond depth 3 SHALL be collapsed by default

#### Scenario: Expand node
- **WHEN** user clicks a collapsed node's expand icon
- **THEN** its direct children SHALL become visible with a smooth transition

### Requirement: Tab navigation
The HTML report SHALL provide tab navigation between three views: Call Tree, Health Dashboard, and Error Timeline.

#### Scenario: Switch tabs
- **WHEN** user clicks a tab header
- **THEN** the corresponding content panel SHALL be displayed and other panels SHALL be hidden

### Requirement: Health dashboard
The health dashboard SHALL display overall subagent health metrics including success rate, health score, total runs, average rounds/tokens/duration, and failure mode breakdown with severity indicators.

#### Scenario: Healthy status
- **WHEN** overall success rate is above 90%
- **THEN** the health dashboard SHALL display a green "Healthy" indicator

#### Scenario: Critical status
- **WHEN** overall success rate is below 50%
- **THEN** the health dashboard SHALL display a red "Critical" indicator with failure mode recommendations

### Requirement: JSON-safe TraceNode serialization
`nodes_to_json()` SHALL serialize `Vec<TraceNode>` into `serde_json::Value`, preserving tree structure, all node fields, and child event arrays, without panicking on any valid UTF-8 content.

#### Scenario: Multi-byte UTF-8 in node content
- **WHEN** a TraceNode label or event data contains multi-byte UTF-8 characters (e.g., box-drawing, CJK, emoji)
- **THEN** `nodes_to_json()` SHALL produce valid JSON without panicking

### Requirement: String truncation is char-boundary safe
All string truncation operations in the trace module SHALL ensure slice boundaries fall on valid UTF-8 character boundaries, preventing panics with multi-byte characters.

#### Scenario: Multi-byte character at truncation boundary
- **WHEN** a string containing multi-byte UTF-8 character '─' (3 bytes) at the truncation cutoff position
- **THEN** the truncation SHALL adjust to the nearest valid char boundary instead of panicking

### Requirement: Failure diagnostics surfaced in trace rendering
The trace rendering (`call_tree`, `error_timeline`, `chrome_trace`, `html`) SHALL surface the structured failure diagnostics when a subagent failed: the `FailureRootCause` category (and guardian reason when applicable), the complete failed tool-call sequence with per-step elapsed time, the truncated failed-round context (assistant text + final tool output), and the retry history.

#### Scenario: Call tree shows failed sequence and root cause
- **WHEN** a failed subagent trace is rendered with `call_tree`
- **THEN** the output SHALL include the root-cause category and the ordered failed tool-call sequence with per-step durations

#### Scenario: Error timeline groups by root cause
- **WHEN** a failed subagent trace is rendered with `error_timeline`
- **THEN** the breakdown SHALL group failures by `FailureRootCause` category and include retry-history entries

#### Scenario: HTML report includes diagnostics section
- **WHEN** a failed subagent trace is rendered with `html`
- **THEN** the report SHALL include a failure-diagnostics section with root cause, failed sequence, failed-round context, and retry history

### Requirement: Raw diagnostics output
The trace rendering SHALL support a raw mode that prints the stored failure diagnostics JSON (root cause, failed sequence, failed-round context, retry history) without rendering, for piping to external tools.

#### Scenario: Raw mode emits diagnostics JSON
- **WHEN** a failed subagent trace is rendered with raw mode
- **THEN** the stored diagnostics SHALL be printed as pretty JSON to stdout

