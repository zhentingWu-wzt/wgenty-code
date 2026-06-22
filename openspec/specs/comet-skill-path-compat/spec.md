# comet-skill-path-compat Specification

## Purpose
TBD - created by archiving change comet-workflow-compat. Update Purpose after archive.
## Requirements
### Requirement: Runtime external skill registry discovers all Comet-compatible roots
The system SHALL discover external skills from a unified set of root directories: project `.wgenty-code/skills/`, user `~/.wgenty-code/skills/`, and user `~/.claude/skills/`. Discovery SHALL be consistent across TUI app startup, daemon tool registry wiring, CLI `skills list`, and TUI completion engine.

#### Scenario: Comet skills installed only in `~/.claude/skills` are discoverable
- **WHEN** Comet skills (comet, comet-open, comet-design, comet-build, comet-verify, comet-archive, comet-hotfix, comet-tweak) are installed in `~/.claude/skills/` but not in `~/.wgenty-code/skills/`
- **THEN** the external skill registry SHALL resolve `/comet` to the Comet skill
- **AND** the `skill` tool SHALL list comet as an available external skill
- **AND** the TUI slash command completion SHALL suggest `/comet` when user types `/com`

#### Scenario: Same skill exists in both roots, wgenty-code wins
- **WHEN** `comet` skill exists in both `~/.wgenty-code/skills/comet/` and `~/.claude/skills/comet/`
- **THEN** the registry SHALL resolve to the `~/.wgenty-code/skills/` version
- **AND** the shadowed `~/.claude/skills/` version SHALL be recorded in the shadowed definitions list

#### Scenario: Skill root does not exist on disk
- **WHEN** `~/.claude/skills/` does not exist on disk
- **THEN** the discovery SHALL silently skip that root without error
- **AND** other roots SHALL still be scanned normally

### Requirement: Unified skill root resolution accessible to all consumers
The system SHALL provide a single `SkillRootResolver` that returns the ordered list of skill roots: project `.wgenty-code/skills/`, user `~/.wgenty-code/skills/`, user `~/.claude/skills/`.

#### Scenario: All consumers see the same root list
- **WHEN** TUI app, daemon state, completion engine, and CLI skills list each request skill roots
- **THEN** all four consumers SHALL receive roots in the same order and count
- **AND** no consumer SHALL hardcode its own root list

### Requirement: Startup logging of discovered skills
The system SHALL log, at session startup, the total number of external skills discovered and the root directories scanned.

#### Scenario: Session starts with skills in multiple roots
- **WHEN** a new TUI session starts and skills exist in `~/.wgenty-code/skills/` and `~/.claude/skills/`
- **THEN** a trace-level log SHALL report the count of skills discovered and which roots were scanned

