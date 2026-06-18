# Comet Design Handoff

- Change: external-skill-runtime
- Phase: design
- Mode: compact
- Context hash: 9c088ff94266e3c52a63b8752c46361cf71bdcfa2b1a3b8038ca7dc5b058ca52

Generated-by: comet-handoff.sh

OpenSpec remains the canonical capability spec. This handoff is a deterministic, source-traceable context pack, not an agent-authored summary.

## openspec/changes/external-skill-runtime/proposal.md

- Source: openspec/changes/external-skill-runtime/proposal.md
- Lines: 1-31
- SHA256: cd8940be369ff13872132d7917a3e25387c4f1a7cd0719fa7f38e1bee677699b

```md
## Why

wgenty-code already has built-in skills, agents, plugin compatibility, and OpenSpec-backed project workflows, but it cannot yet execute Claude Code-style external skills such as `/comet`, `/comet-open`, or `superpowers:brainstorming` as first-class runtime capabilities. Adding a Claude Code-compatible external skill runtime lets wgenty-code reuse mature workflow skills for OpenSpec + Superpowers orchestration without hardcoding Comet into the application.

## What Changes

- Add an external skill discovery layer that loads skills from user, project, and plugin/cache locations.
- Parse skill metadata and markdown instructions into runtime skill definitions.
- Route slash commands such as `/comet ...`, `/opsx:new ...`, and `/superpowers:brainstorming ...` to matching external skills.
- Provide a Skill tool/runtime action that lets the model load nested skills on demand.
- Inject an available-skills summary into prompt context so the model can choose valid skills.
- Resolve duplicate skill names with a deterministic source priority and expose the selected source for debugging.
- Introduce policy hook interfaces around skill load, nested invocation, and guarded execution points so future work can enforce Comet/Superpowers rules in Rust.
- Do not rewrite Comet shell scripts, OpenSpec CLI behavior, or Superpowers skill content in this change.

## Capabilities

### New Capabilities
- `external-skill-runtime`: Discovers, loads, routes, and invokes Claude Code-style external skills with nested skill support and policy hook extension points.

### Modified Capabilities
- `plugin-format-compat`: External skill discovery must interoperate with existing plugin cache/source layout and naming conventions.

## Impact

- `src/knowledge/`: Extend or complement the existing skills framework with external skill definitions, metadata parsing, and registry resolution.
- `src/agent/` / `src/agent_loop` or equivalent runtime loop: Add slash-command routing and Skill tool handling for model-driven skill loading.
- `src/config/` or existing settings layer: Add configurable skill search roots and source priority where appropriate.
- Prompt assembly: Include an available-skills summary and loaded skill instructions without bloating every turn unnecessarily.
- Tool registry / guardian integration: Register Skill as a safe runtime action and keep side-effecting operations delegated to existing tools such as shell/file tools.
- Tests: Add coverage for discovery, metadata parsing, conflict resolution, slash routing, nested invocation, and policy hook emission.
```

## openspec/changes/external-skill-runtime/design.md

- Source: openspec/changes/external-skill-runtime/design.md
- Lines: 1-103
- SHA256: d5812b83812ab45953a9ec2569cdef352f2d659502cef05d068b039791b24771

[TRUNCATED]

```md
## Context

wgenty-code currently has a Rust-native `knowledge` module with built-in skills, a `SkillRegistry`, a simple `SkillLoader` for `SKILL.md`, and a `SkillExecutor` for executable Rust skills. It also has prompt assembly, tool execution, plugin compatibility specs, and OpenSpec-backed planning artifacts. However, Claude Code-style skills are mostly instruction documents: a slash command or Skill tool loads markdown instructions into the conversation, and the model follows those instructions by using ordinary tools. Comet depends on that model-driven style because `/comet` loads child skills such as `/comet-open`, which then load OpenSpec and Superpowers skills and rely on shell scripts for state guards.

This change adds an external skill runtime that is compatible enough with Claude Code-style skills to run workflows such as Comet without hardcoding Comet itself. The runtime should reuse existing wgenty-code primitives where possible: prompt assembly for skill listings and loaded instructions, the tool registry for Skill invocation, and existing plugin/cache conventions for locating external skills.

## Goals / Non-Goals

**Goals:**

- Discover external markdown skills from project, user, and plugin/cache roots.
- Parse skill frontmatter and body into runtime definitions while preserving markdown instructions verbatim.
- Route slash commands to external skills and inject the selected skill instructions into model context.
- Provide a Skill runtime action/tool so the model can load nested skills by name with arguments.
- Inject a compact available-skills list into prompt context without loading every full skill body upfront.
- Resolve duplicate skill names deterministically and expose source information for debugging.
- Add policy hook interfaces for future Rust-enforced workflow constraints.

**Non-Goals:**

- Reimplement Comet state scripts or OpenSpec CLI behavior in Rust.
- Hardcode `/comet` as a special application workflow.
- Fully clone Claude Code's complete tool surface or permission model.
- Enforce every Superpowers/Comet instruction in Rust in the first version.

## Decisions

### Decision 1: Treat external skills as instruction-loaded runtime assets

External skills should be represented separately from executable Rust skills. A new external skill definition should include name, description, namespace/display name, source root, source priority, path, raw frontmatter, and markdown body. Loading an external skill returns a tool/result-style message containing the full instructions and base directory, rather than calling a Rust `Skill::execute` implementation.

Alternative considered: convert every markdown skill into a `Skill` trait implementation. That would blur the boundary between executable skills and instruction skills, and would force markdown workflows into a synchronous execute-return model even though they are meant to steer subsequent model behavior.

### Decision 2: Use two-layer prompt injection

The runtime should keep the current two-layer idea from `SkillLoader`: list names/descriptions up front, load full bodies on demand. The available-skills list should include canonical names such as `comet`, `comet-open`, and `superpowers:brainstorming`, short descriptions, and enough source metadata for debugging when verbose mode is enabled.

Full skill bodies should be injected only when a slash command is invoked or the Skill tool is called. This avoids bloating every prompt with all installed skills while keeping model choice grounded.

### Decision 3: Support Claude Code-style source discovery with deterministic priority

Discovery should scan configured roots in a deterministic order. The first version should support at least:

1. Project-local roots such as `<repo>/.claude/skills` and `<repo>/.codex/skills`.
2. User roots such as `~/.claude/skills` and `~/.codex/skills`.
3. Plugin/cache roots already used by wgenty-code plugin compatibility work.

When two sources provide the same canonical skill name, higher-priority sources win and the losing definitions are retained as shadowed entries for diagnostics. Project-local skills should override user/global/plugin skills so a repository can customize workflows.

### Decision 4: Slash commands are a routing layer over external skill loading

When user input starts with `/`, the runtime should parse the command name and raw argument tail. Built-in commands keep their existing behavior. If no built-in command matches, the external skill registry should resolve the command name. On success, the agent turn starts with the external skill loaded and the raw arguments preserved as `ARGUMENTS` context. On failure, the UI should show an actionable unknown-command message and optionally suggest similar skill names.

This keeps slash commands as user-facing entry points without making each skill a separate compiled command.

### Decision 5: Provide a Skill tool for nested skill invocation

The model needs a first-class Skill action with schema `{ skill: string, args?: string }`. Calling it resolves and loads the requested external skill, returns the skill body as a tool result, and records the loaded skill in the turn/session context. This mirrors Claude Code's nested skill behavior: `/comet` can require `comet-open`, and `comet-open` can require `openspec-explore`.

The Skill tool should be read-only from the guardian perspective because it only loads local instructions. Any side effects remain routed through existing shell/file tools and their permission checks.

### Decision 6: Add policy hooks now, enforce later

The runtime should define policy hook interfaces around skill lifecycle events without making Comet-specific policy mandatory in the first version. Suggested events:

- `before_skill_resolve(name, args, source_context)`
- `after_skill_loaded(definition)`
- `before_nested_skill_call(parent, child)`
- `before_tool_call(tool_name, input, loaded_skills)`
- `before_user_decision(prompt, loaded_skills)`

The default policy should allow all loads and only emit structured events. Future changes can register Comet-aware policies that enforce phase checks, decision-point pauses, debugging gates, or subagent coordination constraints.

### Decision 7: Keep OpenSpec and Comet as external dependencies

Comet workflows should continue to call `openspec` and Comet shell scripts through normal tools. The runtime only makes the skill instructions available and preserves base-directory context so scripts can be located by the skill text. This lowers implementation risk and keeps compatibility with existing skill packages.

## Risks / Trade-offs

- Model-driven instructions can drift from required workflow rules → Mitigation: preserve full skill text, make available-skills accurate, and add policy hook events for later hardening.
```

Full source: openspec/changes/external-skill-runtime/design.md

## openspec/changes/external-skill-runtime/tasks.md

- Source: openspec/changes/external-skill-runtime/tasks.md
- Lines: 1-32
- SHA256: 060b1f0c26cb42129d9659caad7b7f45c8a9dafe9142239640df087156a35aa6

```md
## 1. External Skill Model and Discovery

- [ ] 1.1 Define external skill data structures for metadata, body, source root, priority, shadowed entries, and loaded-skill context.
- [ ] 1.2 Implement discovery for project-local, user-level, and plugin/cache skill roots.
- [ ] 1.3 Parse `SKILL.md` frontmatter and preserve markdown instructions verbatim.
- [ ] 1.4 Implement deterministic conflict resolution and diagnostics for shadowed skill definitions.

## 2. Runtime Integration

- [ ] 2.1 Inject compact available-skills listings into prompt assembly without loading full skill bodies upfront.
- [ ] 2.2 Route slash commands to built-in commands first and external skills second, preserving raw argument text.
- [ ] 2.3 Add a Skill runtime action/tool for nested external skill loading with namespaced skill support.
- [ ] 2.4 Track loaded skills per turn or session and avoid duplicate full-instruction injection.

## 3. Policy Hooks and Safety

- [ ] 3.1 Define policy hook interfaces for skill resolve/load, nested skill calls, tool-call observation, and user-decision observation.
- [ ] 3.2 Provide a permissive default policy implementation that emits structured diagnostics.
- [ ] 3.3 Ensure the Skill runtime action is treated as read-only while side effects remain delegated to existing guarded tools.

## 4. Plugin Compatibility

- [ ] 4.1 Connect external skill discovery to enabled plugin/cache roots that use the existing CC-format cache layout.
- [ ] 4.2 Preserve plugin/cache source metadata in external skill definitions and diagnostics.

## 5. Verification

- [ ] 5.1 Add unit tests for metadata parsing, missing-name fallback, namespaced skill names, and body preservation.
- [ ] 5.2 Add unit tests for source priority, shadowed definitions, and diagnostic output.
- [ ] 5.3 Add integration tests or fixtures for slash routing and nested Skill runtime loading.
- [ ] 5.4 Add tests for plugin/cache skill discovery using a CC-format fixture.
- [ ] 5.5 Run formatting, clippy, and the relevant test suite.
```

## openspec/changes/external-skill-runtime/specs/external-skill-runtime/spec.md

- Source: openspec/changes/external-skill-runtime/specs/external-skill-runtime/spec.md
- Lines: 1-120
- SHA256: 3d80fe04862f84ef5b9b84f8fb4782f2806bd0384509be8a0ff167518f4e3b5f

[TRUNCATED]

```md
## ADDED Requirements

### Requirement: External skill discovery
The system SHALL discover Claude Code-style external skills from configured wgenty-code project, wgenty-code user, plugin/cache, and configured extra skill roots without requiring skills to be compiled into wgenty-code.

#### Scenario: Discover project-local skill
- **WHEN** a repository contains `.wgenty-code/skills/comet/SKILL.md`
- **THEN** the external skill registry includes a `comet` skill sourced from that project-local path

#### Scenario: Discover user skill
- **WHEN** the user home contains `.wgenty-code/skills/superpowers/brainstorming/SKILL.md`
- **THEN** the external skill registry includes a `superpowers:brainstorming` skill sourced from the user path

#### Scenario: Discover plugin cache skill
- **WHEN** an enabled plugin cache contains a `skills/<name>/SKILL.md` entry
- **THEN** the external skill registry includes that skill with plugin/cache source metadata

#### Scenario: Discover configured extra root skill
- **WHEN** settings include an extra skill root containing `skills/custom-flow/SKILL.md`
- **THEN** the external skill registry includes `custom-flow` with configured-root source metadata

#### Scenario: Portable namespace directory maps to canonical name
- **WHEN** a skill exists at `.wgenty-code/skills/superpowers/brainstorming/SKILL.md` without a frontmatter name
- **THEN** the runtime uses `superpowers:brainstorming` as the canonical skill name

### Requirement: Skill metadata parsing
The system SHALL parse external skill frontmatter and body into a runtime definition while preserving the markdown instructions verbatim for model consumption.

#### Scenario: Frontmatter defines name and description
- **WHEN** a `SKILL.md` file contains YAML frontmatter with `name` and `description`
- **THEN** the runtime definition uses those fields for canonical name and available-skill listing
- **AND** the full markdown body remains available for on-demand loading

#### Scenario: Missing name falls back to directory
- **WHEN** a `SKILL.md` file omits `name` in frontmatter
- **THEN** the runtime uses the skill directory name as the canonical skill name

### Requirement: Deterministic skill conflict resolution
The system SHALL resolve duplicate external skill names using a deterministic source priority and retain shadowed definitions for diagnostics.

#### Scenario: Project skill overrides user skill
- **WHEN** both project and user roots define a skill named `comet`
- **THEN** resolving `comet` returns the project-local definition
- **AND** diagnostics can report that the user-level definition was shadowed

#### Scenario: Conflict source is explainable
- **WHEN** debug or verbose skill listing is requested
- **THEN** the system reports the selected source path and any shadowed source paths for duplicate skill names

### Requirement: Available skills prompt listing
The system SHALL inject a compact available-skills listing into model context so the model can choose valid external skills without loading every full skill body upfront.

#### Scenario: Available listing contains discovered skill
- **WHEN** external skills are discovered before a model turn
- **THEN** the prompt context includes each canonical skill name and description
- **AND** it instructs the model to load full instructions through the Skill runtime action when needed

#### Scenario: Full bodies are loaded on demand
- **WHEN** an external skill is present in the available listing but has not been invoked
- **THEN** the prompt context does not include that skill's full markdown body

### Requirement: Slash command skill routing
The system SHALL route slash commands that do not match built-in commands to matching external skills.

#### Scenario: User invokes comet slash command
- **WHEN** the user enters `/comet 如果把 comet 的能力融合进这个项目`
- **THEN** the runtime resolves the `comet` external skill
- **AND** the model turn receives the full `comet` skill instructions
- **AND** the raw argument text is preserved as `ARGUMENTS` context

#### Scenario: Unknown slash command reports suggestions
- **WHEN** the user enters a slash command with no built-in or external skill match
- **THEN** the system reports that the command is unknown
- **AND** it suggests similar external skill names when available

### Requirement: Nested Skill runtime action
The system SHALL provide a Claude Code-compatible `skill`/`Skill` runtime action that allows the model to load another external skill by canonical name with optional arguments.

#### Scenario: Comet loads child skill
- **WHEN** the loaded `comet` skill instructs the model to invoke `comet-open`
```

Full source: openspec/changes/external-skill-runtime/specs/external-skill-runtime/spec.md

## openspec/changes/external-skill-runtime/specs/plugin-format-compat/spec.md

- Source: openspec/changes/external-skill-runtime/specs/plugin-format-compat/spec.md
- Lines: 1-15
- SHA256: eb79c41e0912d0816121e021606918b526637f9c2f850bbf0f34dae5174178dc

```md
## MODIFIED Requirements

### Requirement: REQ-PFC-003 — cache directory structure

MUST MUST 必须支持 `cache/<publisher>/<plugin>/<version>/` 目录结构（除现有扁平结构外），并且外部 skill discovery SHALL be able to discover skill documents below enabled plugin/cache roots that follow this structure.

#### Scenario: CC-format plugin loaded from cache
- GIVEN `cache/anthropic/superpowers/5.1.0/package.json` 存在
- WHEN `PluginManager::load_all()` 运行
- THEN 插件以 CC 格式加载，manifest.source_format = "cc"

#### Scenario: Skill documents discovered from CC-format plugin cache
- **WHEN** `cache/anthropic/superpowers/5.1.0/skills/brainstorming/SKILL.md` exists for an enabled plugin
- **THEN** external skill discovery includes the plugin skill using the canonical name declared by that skill's metadata or directory
- **AND** the skill source metadata identifies the plugin/cache root
```

