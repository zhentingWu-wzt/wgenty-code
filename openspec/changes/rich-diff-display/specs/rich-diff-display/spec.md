## ADDED Requirements

### Requirement: Render unified diff format
The system SHALL render file changes in standard unified diff (`git diff`) format, including hunk headers, context lines, and colored `+`/`-` markers.

#### Scenario: Simple change produces hunk with header
- **WHEN** a file is modified with a single-line change
- **THEN** the diff output SHALL contain a hunk header in `@@ -old_start,old_count +new_start,new_count @@` format
- **AND** the old_start, old_count, new_start, new_count SHALL accurately reflect the changed line ranges

#### Scenario: Context lines surround changes
- **WHEN** a change occurs within a file
- **THEN** up to 3 lines of unchanged context SHALL be shown before and after the changed lines

#### Scenario: Multiple changes far apart produce separate hunks
- **WHEN** two changes are separated by more than 6 unchanged lines (2 × context window)
- **THEN** the diff SHALL produce two separate hunks with independent hunk headers

### Requirement: Word-level diff highlighting
The system SHALL highlight changed words within delete/insert lines to make fine-grained modifications visible.

#### Scenario: Changed word highlighted in delete line
- **WHEN** a line is modified where only a single word changed
- **THEN** the unchanged portion of the delete line SHALL be rendered in the standard delete color
- **AND** the changed word portion SHALL be rendered in a brighter/higher-contrast delete color

#### Scenario: Changed word highlighted in insert line
- **WHEN** a line is modified where only a single word changed
- **THEN** the unchanged portion of the insert line SHALL be rendered in the standard insert color
- **AND** the changed word portion SHALL be rendered in a brighter/higher-contrast insert color

#### Scenario: Identical lines skip word diff
- **WHEN** a paired delete and insert line have identical content
- **THEN** no word-level segments SHALL be computed (standard line rendering used)

### Requirement: Diff statistics summary
The system SHALL display a summary line showing the file path and counts of additions and deletions.

#### Scenario: Stats line shows file path and change counts
- **WHEN** a diff is rendered for a file
- **THEN** the first line SHALL contain the file path prefixed by a chevron marker (▸)
- **AND** SHALL display the count of added lines after a `+` prefix
- **AND** SHALL display the count of deleted lines after a `-` prefix

#### Scenario: No changes shows empty diff indicator
- **WHEN** old and new content are identical
- **THEN** the diff output SHALL display "(no changes detected)"
- **AND** no hunk lines SHALL be rendered

### Requirement: Dual rendering modes
The system SHALL provide two rendering modes: standalone (with line-number gutter) and inline/compact (without gutter).

#### Scenario: Standalone mode shows line number gutter
- **WHEN** diff is rendered in standalone mode
- **THEN** each line SHALL include a gutter with old line number, diff marker (` `, `-`, `+`), and new line number

#### Scenario: Inline mode omits gutter for compact display
- **WHEN** diff is rendered in inline/chat mode
- **THEN** lines SHALL omit the line-number gutter
- **AND** use a `  ` / `- ` / `+ ` prefix directly before content

### Requirement: Line count truncation
The system SHALL enforce maximum line limits to prevent excessive output.

#### Scenario: Standalone view truncated at 50 lines
- **WHEN** a diff exceeds 50 lines in standalone mode
- **THEN** output SHALL be truncated with a "... (N more lines)" indicator

#### Scenario: Inline view truncated at 25 lines
- **WHEN** a diff exceeds 25 lines in inline/chat mode
- **THEN** output SHALL be truncated with a "... (truncated)" indicator
