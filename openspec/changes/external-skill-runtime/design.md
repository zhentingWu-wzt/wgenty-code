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
- Skill discovery across many roots can produce confusing duplicates → Mitigation: deterministic priority, shadowed-entry diagnostics, and clear source display.
- Prompt bloat from too many skills can hurt performance → Mitigation: inject compact listings and load full bodies on demand only.
- External skill markdown may contain unexpected or stale instructions → Mitigation: treat skill content as instructions only after explicit load from trusted local roots; keep side effects behind normal tool permissions.
- Namespace parsing can conflict with slash command syntax → Mitigation: treat the entire slash token after `/` as the canonical skill name, including `:` when present.
- The first version may not fully protect Comet decision points → Mitigation: document this as an intentional non-goal and provide hook boundaries for a follow-up enforcement layer.

## Migration Plan

1. Extend the skill discovery model without removing existing built-in skill APIs.
2. Add external skill registry and loader alongside current `SkillRegistry`/`SkillLoader`.
3. Wire available-skills listing into prompt assembly.
4. Add slash-command routing fallback to external skill resolution.
5. Add Skill tool support for nested loading.
6. Add policy hook interfaces with a permissive default implementation.
7. Add tests and fixtures for project/user/plugin skill roots, namespaced skills, conflicts, slash routing, and nested invocation.

Rollback is straightforward because external skill runtime can be feature-gated or disabled via config, leaving built-in skills and normal chat/tool behavior unchanged.

## Open Questions

- Whether plugin/cache roots should be enabled by default or only when plugin support is configured.
- Whether project-local `.codex/skills` should have the same priority as `.claude/skills`, or whether one should win deterministically when both define the same skill.
- Which UI surface should show loaded-skill/source diagnostics in normal mode versus verbose/debug mode.
