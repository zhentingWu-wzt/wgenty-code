# comet-skill-path-compat Delta Spec

## MODIFIED Requirements

### Requirement: Runtime external skill registry discovers all Comet-compatible roots

The system SHALL move skill root discovery into the generic `SkillManager` in `src/runtime/`. The `SkillRootResolver` SHALL be replaced by `SkillManager::discover()` which reads root paths from runtime configuration. The Comet workflow SHALL NOT require special Rust-level skill path handling.

#### Scenario: Comet skills installed in any configured root are discoverable

- **WHEN** Comet skills are installed in any configured skill root directory
- **THEN** `SkillManager::discover()` SHALL resolve them by their canonical names
- **AND** the TUI slash command completion SHALL suggest `/comet` based on workflow `entry_commands`, not skill path

#### Scenario: Skill root configuration is YAML-driven

- **WHEN** skill roots need to change (add/remove/reorder)
- **THEN** the change SHALL be made in runtime configuration (settings or workflow YAML)
- **AND** SHALL NOT require Rust code changes

### Requirement: Unified skill root resolution accessible to all consumers

The system SHALL replace `SkillRootResolver` with `SkillManager` as the single entry point for skill discovery. All consumers SHALL use `SkillManager::discover()` and `SkillManager::resolve()`.

#### Scenario: All consumers use SkillManager

- **WHEN** TUI app, daemon state, completion engine, and CLI skills list each need skill resolution
- **THEN** all consumers SHALL call `SkillManager` methods
- **AND** no consumer SHALL directly read skill directories or parse SKILL.md files

### Requirement: Startup logging of discovered skills

The system SHALL move startup skill logging into `SkillManager::discover()` with structured log events.

#### Scenario: Session starts with skills in multiple roots

- **WHEN** `SkillManager::discover()` scans multiple configured roots
- **THEN** a `RuntimeEvent::SkillDiscovery` event SHALL be emitted with count per root
- **AND** the existing trace-level log behavior SHALL be preserved
