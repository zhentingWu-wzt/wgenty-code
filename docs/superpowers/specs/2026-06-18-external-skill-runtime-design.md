---
comet_change: external-skill-runtime
role: technical-design
canonical_spec: openspec
---

# External Skill Runtime Design

## Context

wgenty-code already has three important building blocks for a Claude Code-style skill runtime:

- `src/knowledge/loader.rs` scans `skills/*/SKILL.md` and supports a two-layer model: list available skills first, load full instructions on demand.
- `src/tools/meta/load_skill.rs` exposes a read-only tool that returns either the available skill list or one skill's markdown body.
- `src/prompts/mod.rs` injects `skills_inventory` into prompt context without loading every full skill body.

The missing piece is compatibility with richer external instruction skills such as Comet and Superpowers. These skills are not executable Rust functions. They are markdown instruction documents that steer the model to call ordinary tools, invoke nested skills, pause at decision points, and run external commands such as OpenSpec or Comet shell scripts.

The design therefore extends the existing skill system instead of replacing it. Built-in Rust skills remain executable `Skill` trait implementations. External skills become instruction-loaded runtime assets with source metadata, deterministic conflict resolution, slash-command routing, nested loading, and policy hook extension points.

## Goals

- Discover external skills from wgenty-code project/user roots, enabled plugin cache roots, and configured extra roots.
- Preserve the existing two-layer skill injection strategy: compact available listing first, full markdown on demand.
- Route `/comet ...`-style slash commands to external skills when no built-in command matches.
- Add a Claude Code-compatible `skill`/`Skill` runtime action for nested skill loading.
- Track loaded skills and nested invocations for diagnostics and future policy enforcement.
- Provide policy hook interfaces now, while keeping the first version permissive and model-driven.

## Non-Goals

- Do not hardcode Comet as a compiled workflow.
- Do not rewrite OpenSpec CLI, Comet shell scripts, or Superpowers skill content in Rust.
- Do not fully clone Claude Code's complete tool surface or permission system.
- Do not enforce every Comet/Superpowers process rule in Rust in the first version.

## Architecture

```text
user input: /comet ...
  ├── built-in slash command first
  └── external skill fallback
        ├── ExternalSkillRegistry
        │     - discover roots
        │     - parse metadata
        │     - resolve priority
        │     - retain shadowed diagnostics
        ├── skill/Skill runtime action
        │     - nested load
        │     - args passthrough
        │     - max depth 8
        ├── prompt injection
        │     - compact listing
        │     - full body only when loaded
        └── PolicyHooks
              - default allow
              - structured diagnostics
              - future Comet enforcement
```

The implementation should live primarily in `src/knowledge/` and reuse the existing `load_skill` and prompt inventory paths. Suggested module split:

- `knowledge::external`: external skill data model.
- `knowledge::external_registry`: discovery, parsing, priority resolution, diagnostics.
- `knowledge::policy`: policy hook traits, events, and default allow policy.
- `tools::meta::skill`: Claude Code-compatible `skill`/`Skill` runtime action.
- `tools::meta::load_skill`: retained as legacy/internal compatibility, backed by the same registry where possible.

## Data Model

External instruction skills should be modeled separately from executable Rust skills:

```text
ExternalSkillDefinition
  canonical_name: String
  display_name: String
  description: String
  body: String
  frontmatter: SkillFrontmatter
  source: ExternalSkillSource
  source_path: PathBuf
  base_dir: PathBuf
  priority: SkillSourcePriority
  shadowed: Vec<ShadowedSkillDefinition>
```

The body is the full `SKILL.md` content and should be preserved verbatim for model consumption. The loader may parse common frontmatter fields such as `name` and `description`, but unknown fields should be retained or ignored safely rather than causing incompatibility.

`ExternalSkillSource` should include at least:

- `ProjectWgentyCode { root }`
- `UserWgentyCode { root }`
- `PluginCache { plugin_name, version, root }`
- `Configured { label, root }`

This source metadata powers diagnostics, policy hooks, and future trust decisions.

## Discovery and Priority

The first version uses wgenty-code-native roots as the default convention:

1. `repo/.wgenty-code/skills`
2. `~/.wgenty-code/skills`
3. enabled plugin cache skill roots
4. configured extra roots

`.claude` or `.codex` roots are not default roots in this design. They can still be added through configured extra roots for compatibility.

Discovery supports the standard layout:

```text
<root>/skills/<skill>/SKILL.md
```

It also supports portable namespace layout:

```text
<root>/skills/superpowers/brainstorming/SKILL.md
→ canonical_name = "superpowers:brainstorming"
```

Canonical name resolution:

1. Use frontmatter `name` if present.
2. Otherwise use the one-level directory name.
3. For portable two-level namespace layout, infer `<namespace>:<skill>`.

When duplicate canonical names are found, the highest-priority source wins and lower-priority definitions are retained as shadowed diagnostics. A debug listing should be able to explain both the selected source and shadowed sources.

## Slash Routing

Slash command routing should be layered:

1. Parse `/command raw args`.
2. Try existing built-in slash command handling first.
3. If no built-in command matches, resolve `command` in `ExternalSkillRegistry`.
4. If found, load the skill body into the model turn and preserve raw args as `ARGUMENTS`.
5. If not found, return an unknown-command message with similar skill suggestions.

Built-in commands must remain higher priority than external skills to avoid accidental override of core behavior.

The loaded skill context should include:

```text
Base directory for this skill: <base_dir>

<skill markdown body>

ARGUMENTS: <raw args>
```

This shape is important because Comet/Superpowers skills often refer to their base directory and arguments explicitly.

## Skill Runtime Action

The first version should add a Claude Code-compatible `skill`/`Skill` runtime action while keeping `load_skill` as a compatibility path. The new action has schema equivalent to:

```json
{
  "type": "object",
  "properties": {
    "skill": { "type": "string" },
    "args": { "type": "string" }
  },
  "required": ["skill"]
}
```

The action resolves the canonical skill name, applies policy hooks, loads the full instructions, records the invocation, and returns a markdown tool-style result. Namespaced names such as `superpowers:brainstorming` are treated as exact canonical names, not as path separators.

Nested skill depth is limited to 8. When a load would exceed that depth, the runtime denies the request with an actionable maximum-depth error and does not inject the requested body.

Duplicate loads in the same turn should not duplicate the same full markdown body, but they should still be recorded as invocation events for diagnostics.

## Loaded Skill Context

Runtime state should track loaded skills per turn or session:

```text
LoadedSkillRecord
  name: String
  source_path: PathBuf
  base_dir: PathBuf
  args: Option<String>
  parent: Option<String>
  depth: usize
  turn_id: usize
```

This supports:

- duplicate load prevention,
- nested parent/child diagnostics,
- future policy checks,
- user-visible debugging of why a model followed a given workflow.

## Policy Hooks

Policy hooks are designed now but permissive by default. Suggested trait shape:

```text
SkillPolicy
  before_skill_resolve(event) -> PolicyDecision
  after_skill_loaded(event) -> PolicyDecision
  before_nested_skill_call(event) -> PolicyDecision
  before_tool_call_observed(event) -> PolicyDecision
```

`PolicyDecision` supports:

- `Allow`
- `Warn { message }`
- `Deny { message }`

The default policy always allows and emits structured diagnostics. Future policies can enforce Comet rules, such as reading `.comet.yaml` before each operation, denying source writes in `open`/`design`, requiring debugging gates after failures, or blocking build progression until required state fields are set.

## Plugin Cache Integration

Plugin cache roots participate as a lower-priority skill source. For a CC-format plugin cache like:

```text
cache/anthropic/superpowers/5.1.0/
  package.json
  skills/brainstorming/SKILL.md
```

external discovery should include the skill and annotate source metadata as plugin/cache-derived. This should reuse existing plugin compatibility concepts where practical, but instruction skill loading should remain separate from plugin command execution.

## Error Handling

The runtime should distinguish these cases:

- Skill not found: return unknown skill with suggestions.
- Duplicate skill: resolve by priority and expose shadowed diagnostics.
- Invalid frontmatter: skip or degrade gracefully with diagnostics, depending on severity.
- Unreadable file: skip and record a diagnostic.
- Policy denial: stop the operation and return the denial message.
- Nested depth overflow: deny with a maximum-depth error.

Side effects remain behind existing tools and guardian checks. Loading a skill is read-only.

## Testing Strategy

Unit tests:

- Frontmatter name/description parsing.
- Missing name fallback.
- Raw markdown body preservation.
- Source path and base directory correctness.
- Portable namespace mapping.
- Source priority and shadowed diagnostics.
- Default allow and deny policy behavior.
- Nested depth limit.

Runtime/tool tests:

- `skill({ skill: "comet" })` returns markdown instructions.
- `skill({ skill: "superpowers:brainstorming" })` resolves namespaced skills.
- Missing skill returns suggestions.
- Duplicate loads in one turn are idempotent for body injection.

Routing tests:

- `/comet abc` falls back to external skill when no built-in command matches.
- Built-in slash commands still win over external skill names.
- Unknown slash commands suggest similar skills.

Plugin fixture tests:

- CC-format plugin cache containing `skills/*/SKILL.md` is discovered.
- Plugin cache source metadata includes plugin name/version/root.

Regression checks:

- Existing built-in skills tests still pass.
- Prompt skill inventory remains compact.
- `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, and relevant tests pass.

## Implementation Sequence

1. Add external skill data model and source priority types.
2. Extend discovery to `.wgenty-code` roots and configured/plugin roots.
3. Add conflict resolution and diagnostics.
4. Wire external skill inventory into existing prompt skill listing.
5. Add `skill`/`Skill` runtime action backed by the external registry.
6. Add slash command fallback to external skill resolution.
7. Add loaded skill context and nested depth enforcement.
8. Add policy hook interfaces and default allow implementation.
9. Add plugin cache discovery fixtures and tests.
10. Run formatting, clippy, and tests.
