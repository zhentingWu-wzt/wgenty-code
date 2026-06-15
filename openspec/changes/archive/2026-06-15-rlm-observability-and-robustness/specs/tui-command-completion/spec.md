# tui-command-completion Specification

## Purpose
Enable users to interactively select and invoke skills and plugin commands directly from the TUI input box, without memorizing slash-command syntax.

## ADDED Requirements

### Requirement: Skills completion triggered by @ prefix
The TUI input box SHALL trigger a skill completion panel when the user types `@` followed by zero or more characters. The completion panel SHALL list all available skills whose names contain the typed substring (case-insensitive).

#### Scenario: User types @ to see all skills
- **WHEN** user types `@` in the input box
- **THEN** an inline completion panel SHALL appear showing all available skill names sorted alphabetically

#### Scenario: User filters skills by typing partial name
- **WHEN** user types `@com` in the input box
- **THEN** the completion panel SHALL filter to show skills containing "com" (e.g., "comet", "comet-open", "comet-build")

#### Scenario: User selects a skill from the panel
- **WHEN** user navigates to a skill name with arrow keys and presses Enter
- **THEN** the input box SHALL replace `@com` with the full skill invocation syntax (e.g., `/comet-open`)

#### Scenario: User dismisses completion panel
- **WHEN** the completion panel is visible and user presses Escape
- **THEN** the panel SHALL close and the `@` prefix text SHALL remain in the input box unchanged

### Requirement: Plugin command completion triggered by / prefix
The TUI input box SHALL trigger a plugin command completion panel when the user types `/` at the beginning of input (or after whitespace). The panel SHALL list all registered plugin commands.

#### Scenario: User types / to see plugin commands
- **WHEN** user types `/` at the start of the input box
- **THEN** an inline completion panel SHALL appear showing all registered plugin command names with their descriptions

#### Scenario: User filters plugin commands by typing partial name
- **WHEN** user types `/code-` in the input box
- **THEN** the completion panel SHALL filter to show plugin commands starting with "code-" (e.g., "code-review")

#### Scenario: Plugin command with required arguments
- **WHEN** user selects a plugin command that requires arguments
- **THEN** the input box SHALL populate with the command name followed by a space, and the panel SHALL show argument hints

### Requirement: Completion panel provides keyboard navigation
The completion panel SHALL support keyboard navigation consistent with the rest of the TUI.

#### Scenario: Navigate options with arrow keys
- **WHEN** the completion panel is visible
- **THEN** Up/Down arrow keys SHALL move the selection highlight; Tab SHALL move forward, Shift+Tab SHALL move backward

#### Scenario: Cycle through options
- **WHEN** user presses Tab past the last option
- **THEN** selection SHALL wrap to the first option

### Requirement: Completion data sources
The TUI SHALL source skill names from the local skills directory and plugin commands from the PluginRegistry.

#### Scenario: Skills directory scanned at startup
- **WHEN** the TUI starts
- **THEN** all directory names under `~/.claude/skills/` SHALL be loaded as available skill names

#### Scenario: Plugin commands loaded from registry
- **WHEN** plugins are loaded
- **THEN** all registered plugin commands with their descriptions SHALL be available for completion
