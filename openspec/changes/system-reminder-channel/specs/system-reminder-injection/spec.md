## ADDED Requirements

### Requirement: System reminder block injection per user turn

The system SHALL inject a `<system-reminder>` block at the head of every user message sent to the model. The block SHALL appear before the user's actual prompt text in the same message payload.

#### Scenario: First turn injection
- **WHEN** the user submits any prompt for the first time in a session
- **THEN** the outgoing request to the model SHALL include a user message whose content begins with a `<system-reminder>` block followed by the user's prompt text

#### Scenario: Subsequent turn injection
- **WHEN** the user submits a second or later prompt in the same session
- **THEN** the outgoing request SHALL again contain the `<system-reminder>` block at the head of the new user message
- **AND** the reminder content SHALL be re-evaluated from current file sources (not cached from the first turn)

#### Scenario: System prompt remains clean
- **WHEN** the reminder block is constructed
- **THEN** none of the reminder content SHALL appear in any `ChatMessage::system` of the system prompt chain
- **AND** the `system_messages` Vec returned by the prompt assembler SHALL NOT contain `# AGENTS.md` or `# WGENTY.md — 项目规则与约定` layers

---

### Requirement: Four content source layers in deterministic order

The reminder block SHALL aggregate content from up to four file sources, in this exact order: user-global instructions, user-global rules, project instructions, project agent conventions.

#### Scenario: All four sources present
- **WHEN** `~/.wgenty-code/WGENTY.md` exists, `~/.wgenty-code/rules/*.md` contains at least one file, project root `WGENTY.md` exists, and project root `AGENTS.md` exists
- **THEN** the reminder block SHALL include, in this order: user-global WGENTY content, then each `rules/*.md` file in alphabetical order by filename, then project WGENTY content, then project AGENTS content

#### Scenario: User global WGENTY.md missing
- **WHEN** `~/.wgenty-code/WGENTY.md` does not exist
- **THEN** the user-global instructions section SHALL be omitted from the reminder block
- **AND** no empty heading, placeholder, or error indicator SHALL appear in its place
- **AND** the remaining sections SHALL render in their normal order without gaps

#### Scenario: User rules directory missing or empty
- **WHEN** `~/.wgenty-code/rules/` does not exist, or exists but contains no `*.md` files
- **THEN** the user-global rules section SHALL be omitted from the reminder block

#### Scenario: Project WGENTY.md missing
- **WHEN** the project root `WGENTY.md` does not exist
- **THEN** the project instructions section SHALL be omitted from the reminder block

#### Scenario: Project AGENTS.md missing
- **WHEN** the project root `AGENTS.md` does not exist
- **THEN** the project agent conventions section SHALL be omitted from the reminder block

#### Scenario: All sources missing
- **WHEN** none of the four file sources exist
- **THEN** the reminder block SHALL be omitted entirely (no preamble, no closing, no empty `<system-reminder>` tags)
- **AND** the user message SHALL be sent as if no reminder mechanism existed

---

### Requirement: Rules directory alphabetical ordering

When multiple files exist under `~/.wgenty-code/rules/`, the system SHALL include them in case-sensitive byte-wise ascending filename order.

#### Scenario: Multiple rule files
- **WHEN** `~/.wgenty-code/rules/` contains `comet-phase-guard.md`, `apple.md`, and `zebra.md`
- **THEN** the reminder block SHALL include them in this order: `apple.md`, `comet-phase-guard.md`, `zebra.md`

#### Scenario: Non-markdown files in rules directory
- **WHEN** `~/.wgenty-code/rules/` contains a file `notes.txt` alongside `foo.md`
- **THEN** the reminder block SHALL include only `foo.md`
- **AND** `notes.txt` SHALL be ignored without error

#### Scenario: Subdirectories in rules directory
- **WHEN** `~/.wgenty-code/rules/` contains a subdirectory `archive/old.md`
- **THEN** the reminder block SHALL ignore the subdirectory and its contents
- **AND** only top-level `*.md` files SHALL be considered

---

### Requirement: Source attribution header per section

Each content section in the reminder block SHALL be preceded by a `Contents of <absolute-path> (<description>):` header line that identifies its source.

#### Scenario: User-global WGENTY.md attribution
- **WHEN** the reminder block includes user-global WGENTY content
- **THEN** the section SHALL begin with `Contents of /Users/<user>/.wgenty-code/WGENTY.md (user's private global instructions for all projects):` (path expanded to the user's actual home directory)

#### Scenario: User rules file attribution
- **WHEN** the reminder block includes a rule file `~/.wgenty-code/rules/foo.md`
- **THEN** that file's section SHALL begin with `Contents of /Users/<user>/.wgenty-code/rules/foo.md (user's private global instructions for all projects):`

#### Scenario: Project WGENTY.md attribution
- **WHEN** the reminder block includes project WGENTY content
- **THEN** the section SHALL begin with `Contents of <project-root-absolute-path>/WGENTY.md (project instructions, checked into the codebase):`

#### Scenario: Project AGENTS.md attribution
- **WHEN** the reminder block includes project AGENTS content
- **THEN** the section SHALL begin with `Contents of <project-root-absolute-path>/AGENTS.md (project agent conventions, checked into the codebase):`

---

### Requirement: Double preamble wrapping

The reminder block SHALL open with a high-priority preamble and close with a relevance-disclaimer preamble, framing the content as authoritative-but-contextual.

#### Scenario: Opening preamble present
- **WHEN** the reminder block is non-empty
- **THEN** immediately after the opening `<system-reminder>` tag and before any source section, the block SHALL contain a preamble line stating: `As you answer the user's questions, you can use the following context:` followed by a `# claudeMd`-equivalent header and the statement `Codebase and user instructions are shown below. Be sure to adhere to these instructions. IMPORTANT: These instructions OVERRIDE any default behavior and you MUST follow them exactly as written.`

#### Scenario: Closing preamble present
- **WHEN** the reminder block is non-empty
- **THEN** immediately before the closing `</system-reminder>` tag and after all source sections, the block SHALL contain a preamble stating: `IMPORTANT: this context may or may not be relevant to your tasks. You should not respond to this context unless it is highly relevant to your task.`

#### Scenario: Preambles align with Claude Code wording 1:1
- **WHEN** the reminder block is rendered
- **THEN** the wording of both preambles SHALL match Claude Code's reference text exactly (no paraphrasing, no localization of the English preamble strings)

---

### Requirement: Token budget calculation and one-time warning

The system SHALL compute the approximate token cost of the entire reminder block (all sections + preambles + attribution headers) and emit a single warning per session when the total exceeds the configured threshold.

#### Scenario: Reminder block under threshold
- **WHEN** the total reminder content is below the configured token threshold
- **THEN** no warning SHALL be emitted

#### Scenario: Reminder block exceeds threshold on first turn
- **WHEN** the reminder block exceeds the configured token threshold on the first user turn
- **THEN** the system SHALL emit exactly one warning to the TUI status area indicating estimated tokens and contributing file sizes
- **AND** the request SHALL still proceed (warning is informational, not blocking)

#### Scenario: Threshold-exceeding block on subsequent turns
- **WHEN** the reminder block exceeds the threshold on the second or later turn in the same session, and the warning already fired
- **THEN** no additional warning SHALL be emitted in that session

#### Scenario: Threshold computed across all sources
- **WHEN** the budget calculation runs
- **THEN** it SHALL sum the byte/token cost of all four source layers plus preamble overhead
- **AND** the calculation SHALL NOT skip any included section

---

### Requirement: Reminder injection scope limited to main session

The reminder block SHALL only be injected into the main interactive session's user messages. Subagent sessions SHALL NOT receive the reminder block.

#### Scenario: Main session injection
- **WHEN** the main TUI/REPL session sends a user message
- **THEN** the reminder block SHALL be present

#### Scenario: Subagent session exclusion
- **WHEN** a subagent (spawned via Task tool or agent runtime) constructs its own user messages
- **THEN** those messages SHALL NOT contain the `<system-reminder>` block
- **AND** the subagent's prompt construction path SHALL NOT call the reminder builder

---

### Requirement: Existing PromptContextBuilder API surface preserved

The system SHALL preserve the public method signatures `PromptContextBuilder::with_wgenty_md(...)` and `PromptContextBuilder::with_agents_md(...)`. Their internal effect is reassigned to populate reminder sources rather than push system messages.

#### Scenario: Existing callers compile without modification
- **WHEN** existing code that calls `with_wgenty_md(sections)` or `with_agents_md(sections)` is recompiled against the new implementation
- **THEN** the call sites SHALL compile without source changes

#### Scenario: Behavioral change is internal
- **WHEN** an existing call passes sections to `with_wgenty_md`
- **THEN** those sections SHALL participate in the reminder block (project WGENTY layer) and SHALL NOT produce a `# WGENTY.md — 项目规则与约定` system message
