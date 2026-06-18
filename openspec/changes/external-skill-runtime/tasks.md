## 1. External Skill Model and Discovery

- [x] 1.1 Define external skill data structures for metadata, body, source root, priority, shadowed entries, and loaded-skill context.
- [x] 1.2 Implement discovery for project-local, user-level, and plugin/cache skill roots.
- [x] 1.3 Parse `SKILL.md` frontmatter and preserve markdown instructions verbatim.
- [x] 1.4 Implement deterministic conflict resolution and diagnostics for shadowed skill definitions.

## 2. Runtime Integration

- [x] 2.1 Inject compact available-skills listings into prompt assembly without loading full skill bodies upfront.
- [x] 2.2 Route slash commands to built-in commands first and external skills second, preserving raw argument text.
- [x] 2.3 Add a Skill runtime action/tool for nested external skill loading with namespaced skill support.
- [x] 2.4 Track loaded skills per turn or session and avoid duplicate full-instruction injection.

## 3. Policy Hooks and Safety

- [x] 3.1 Define policy hook interfaces for skill resolve/load, nested skill calls, tool-call observation, and user-decision observation.
- [x] 3.2 Provide a permissive default policy implementation that emits structured diagnostics.
- [x] 3.3 Ensure the Skill runtime action is treated as read-only while side effects remain delegated to existing guarded tools.

## 4. Plugin Compatibility

- [x] 4.1 Connect external skill discovery to enabled plugin/cache roots that use the existing CC-format cache layout.
- [x] 4.2 Preserve plugin/cache source metadata in external skill definitions and diagnostics.

## 5. Verification

- [x] 5.1 Add unit tests for metadata parsing, missing-name fallback, namespaced skill names, and body preservation.
- [x] 5.2 Add unit tests for source priority, shadowed definitions, and diagnostic output.
- [x] 5.3 Add integration tests or fixtures for slash routing and nested Skill runtime loading.
- [x] 5.4 Add tests for plugin/cache skill discovery using a CC-format fixture.
- [x] 5.5 Run formatting, clippy, and the relevant test suite.
