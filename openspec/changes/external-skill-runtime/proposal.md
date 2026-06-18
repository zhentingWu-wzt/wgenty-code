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
