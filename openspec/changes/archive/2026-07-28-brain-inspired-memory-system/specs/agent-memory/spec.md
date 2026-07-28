## MODIFIED Requirements

### Requirement: Memory storage via MemoryManager

All memories SHALL be stored exclusively via `MemoryManager`, using its per-file Storage backend. Memories SHALL be physically separated by scope:
- **Project memories** SHALL be stored at `<project_root>/.wgenty-code/memory/<id>.json`
- **Global memories** SHALL be stored at `~/.wgenty-code/memory/<id>.json`

`project_root` SHALL equal the current working directory (CWD), with no upward search for project markers. Each memory SHALL use the `context::MemoryEntry` type with fields: id, memory_type, content, timestamp, importance, tags, metadata, AND feedback-tracking fields `recall_count`, `hit_count`, `last_reinforced_at` (Option; None means decay anchors at `timestamp`), `superseded_by` (Option; id of the superseding memory), and `stale_marked_at` (Option; set when codebase-staleness has been applied). Feedback and staleness fields SHALL deserialize with defaults (`recall_count=0`, `hit_count=0`, `last_reinforced_at=None`, `superseded_by=None`, `stale_marked_at=None`) when absent so existing memory JSON loads without migration. The memory file's filename SHALL remain the stable `id` (UUID); semantic display slugs SHALL NOT replace id-as-filename. `MemoryManager` SHALL track each loaded memory's origin (Project or Global) and persist memories to the directory matching their scope.

#### Scenario: Project memory persisted to project-local directory

- **WHEN** `MemoryManager::add_memory(entry, Project)` is called with a valid MemoryEntry
- **THEN** the entry is saved as `<CWD>/.wgenty-code/memory/<id>.json`

#### Scenario: Global memory persisted to global directory

- **WHEN** `MemoryManager::add_memory(entry, Global)` is called with a valid MemoryEntry
- **THEN** the entry is saved as `~/.wgenty-code/memory/<id>.json`

#### Scenario: CWD unavailable degrades to global storage

- **WHEN** the project-local memory directory cannot be created (e.g. CWD deleted or unwritable)
- **THEN** project memories SHALL fall back to the global memory directory and a warning SHALL be logged

#### Scenario: CWD equals home directory

- **WHEN** `project_root` resolves to the user's home directory (project root coincides with global root)
- **THEN** project memories SHALL be written to the global memory directory (merged pool) and a warning SHALL be logged

#### Scenario: Legacy memory JSON loads with feedback-field defaults

- **WHEN** a memory JSON file written before this change (lacking feedback/staleness fields) is loaded
- **THEN** it deserializes successfully with `recall_count=0`, `hit_count=0`, `last_reinforced_at=None`, `superseded_by=None`, and `stale_marked_at=None`

### Requirement: Memory recall at session startup

At session startup, `MemoryManager::load()` SHALL load project memories from `<CWD>/.wgenty-code/memory/` and global memories from `~/.wgenty-code/memory/`. `MemoryManager::search_memories(query)` SHALL retrieve only project memories matching the query via the TF-IDF index (global memories are not indexed and are injected verbatim every turn). Recall ranking, threshold filtering, and global-memory soft-cap ordering SHALL use **effective importance** (see "Effective importance evaluation"). A superseded memory (`superseded_by` is Some) SHALL be excluded from recall. When project memories are successfully selected for injection into a `<memory-context>` block, each such memory's `recall_count` SHALL be incremented by one and persisted (so hit-rate damping observes real injection frequency). Global memories injected via `<global-memory>` are outside this counter. When `exploration_epsilon` is greater than zero, recall MAY with that probability replace the lowest-ranked injected project memory with a low-effective-importance project memory not recently recalled (see "Recall exploration injection"). When `exploration_epsilon` is 0 (the default), recall SHALL return the plain effective-importance ranking with no exploration replacement. CLI/`list_memories` style listing SHALL order (and apply minimum-score filters) by effective importance; superseded memories MAY still appear in listings with effective importance 0 for auditability, even though they are excluded from recall injection.

#### Scenario: Global memories injected every turn

- **WHEN** a turn is processed and global memories exist in `~/.wgenty-code/memory/`
- **THEN** a `<global-memory>` block containing all global memories (sorted by effective importance, capped at 50) is injected into the system prompt between the Environment and Skills layers

#### Scenario: Project memories recalled by keyword

- **WHEN** a user message is processed and project memories match the extracted keywords with effective importance >= threshold
- **THEN** a `<memory-context>` block containing the matched project memories is injected (global memories are excluded from this block)

#### Scenario: No global memories

- **WHEN** a turn is processed but no global memories exist
- **THEN** no `<global-memory>` block is injected

#### Scenario: Global memory soft cap exceeded

- **WHEN** more than 50 global memories exist
- **THEN** only the top 50 by effective importance are injected and a warning is logged

#### Scenario: Superseded memory excluded from recall

- **WHEN** a memory has `superseded_by = Some(other_id)` and would otherwise match the recall query
- **THEN** it is not included in the `<memory-context>` block (its effective importance is treated as 0)

#### Scenario: Injected project memories increment recall_count

- **WHEN** one or more project memories are injected into a `<memory-context>` block for a turn
- **THEN** each injected project memory has its `recall_count` increased by one and the updated entry is persisted

#### Scenario: List ordering uses effective importance

- **WHEN** memories are listed via `list_memories` (or equivalent CLI) with a minimum score filter
- **THEN** ordering and the minimum filter use effective importance, and a superseded memory may still appear with effective importance 0

### Requirement: Time-gated memory consolidation

`AutoDreamService::check_and_run()` SHALL be called at session startup before recall, in both TUI/daemon and headless modes. The gate thresholds SHALL be `min_hours = 1` and `min_sessions = 1`. The session-scan throttle SHALL be 10 minutes.

AutoDream SHALL NOT maintain its own disk-based consolidation lock. Cross-process mutual exclusion SHALL be provided solely by `MemoryManager::consolidate()`'s internal `ConsolidationFileLock` (at `~/.wgenty-code/memory/.consolidation.lock`). AutoDream's in-memory `is_consolidating` flag SHALL be reset on each `check_and_run` invocation.

`MemoryManager::consolidate()` does not invoke any LLM call -- it is pure local computation (TF-IDF similarity merge, TTL decay using effective importance for retention, orphan-file reconcile, index rebuild, idempotent codebase-staleness marking, and optional first-run `last_reinforced_at` anchoring). This is the premise that permits the aggressive 1h/1session gate. The retention decision (`should_keep`) SHALL use **effective importance** instead of raw `importance`. `consolidate()` SHALL NOT call an LLM for contradiction resolution, replay, or restatement.

#### Scenario: Consolidation gate passes

- **WHEN** session starts and 1 hour has passed with >= 1 new session and no active consolidation lock held by another process
- **THEN** `MemoryManager::consolidate()` is called, deduplicating and merging similar memories using effective importance for retention

#### Scenario: Consolidation gate fails on time

- **WHEN** session starts but less than 1 hour has passed since last consolidation
- **THEN** consolidation is skipped and the session continues with existing memories

#### Scenario: Consolidation gate fails on session-scan throttle

- **WHEN** session starts within the 10-minute session-scan interval since the last scan
- **THEN** consolidation is skipped without re-scanning the sessions directory

#### Scenario: Cross-process mutual exclusion via MemoryManager lock

- **WHEN** AutoDream triggers `consolidate()` while a concurrent `memory dream` invocation already holds the `ConsolidationFileLock`
- **THEN** AutoDream's `consolidate()` waits on the same lock (no separate AutoDream lock file is created) and no race occurs

#### Scenario: AutoDream does not write a separate disk lock

- **WHEN** AutoDream runs consolidation
- **THEN** no `~/.wgenty-code/.consolidation.lock` (timestamp lock) file is written; only `~/.wgenty-code/.autodream_state.json` (state) and `~/.wgenty-code/memory/.consolidation.lock` (mm internal lock) are touched

#### Scenario: Headless mode triggers AutoDream startup check

- **WHEN** a headless/CLI session starts
- **THEN** `AutoDreamService::check_and_run()` is invoked (fire-and-forget) before the agent loop, identical to TUI startup

#### Scenario: Daemon mode triggers AutoDream startup check

- **WHEN** a TUI/daemon session starts and the daemon process initializes its `DaemonState`
- **THEN** the daemon constructs `AutoDreamService` (with the daemon's `MemoryManager`) and invokes `check_and_run()` (fire-and-forget), so TUI/daemon mode triggers consolidation at session startup

#### Scenario: TUI app does not directly start AutoDream

- **WHEN** a TUI session starts
- **THEN** the TUI app does NOT construct or invoke `AutoDreamService` itself; AutoDream startup is handled solely by the daemon (avoiding duplicate consolidation triggers)

#### Scenario: Consolidation is LLM-free

- **WHEN** `check_and_run()` gates pass and `consolidate()` runs
- **THEN** no LLM call is made; consolidation completes via local TF-IDF merge, TTL/retention using effective importance, orphan reconcile, idempotent codebase-staleness marking, optional anchor migration, and index rebuild

#### Scenario: First consolidate anchors missing last_reinforced_at

- **WHEN** `consolidate()` runs and a memory has `last_reinforced_at = None`
- **THEN** that field is set to the consolidate timestamp once and persisted so subsequent decays do not treat legacy memories as extremely old relative to "now" without an explicit anchor

### Requirement: Proactive memory capture via tool

The system SHALL provide a `memory_add` tool that allows the agent to proactively write a memory entry at any point during a conversation, without waiting for context compaction. The tool SHALL accept parameters: `content` (required string), `memory_type` (enum: Knowledge/Preference/Session/Conversation/Task/Error/Insight/Decision), `scope` (project|global, default project), and optional `importance`. When the new memory is similar (Jaccard >= 0.6) to an existing memory in the same scope, `MemoryManager::add_memory()` SHALL classify the relation as Compatible, Contradicts, or Ambiguous (see "Contradiction detection and supersede resolution") instead of unconditionally merging: Compatible relations merge AND reinforce the existing memory; Contradicts relations supersede the existing memory (tombstone, not hard delete); Ambiguous relations merge AND flag the pair in metadata for possible later resolution (no LLM call in this requirement).

#### Scenario: Agent proactively writes a project memory

- **WHEN** the agent calls `memory_add` with content "note_edit tool uses NoteStore but is registered with store:None, so it doesn't persist", memory_type "Knowledge", scope "project"
- **THEN** `MemoryManager::add_memory()` is called with a `MemoryEntry` of type Knowledge and `MemoryOrigin::Project`, and the memory is saved to `<CWD>/.wgenty-code/memory/<id>.json`

#### Scenario: Agent proactively writes a global memory

- **WHEN** the agent calls `memory_add` with content "Always read actual settings.json before assuming config defaults", scope "global"
- **THEN** `MemoryManager::add_memory()` is called with `MemoryOrigin::Global`, and the memory is saved to `~/.wgenty-code/memory/<id>.json`

#### Scenario: Compatible similar memory merges and reinforces

- **WHEN** the agent calls `memory_add` with content that has >= 0.6 similarity to an existing memory and the relation is classified Compatible
- **THEN** `MemoryManager::add_memory()` merges the new content into the existing memory entry, increments the existing memory's `hit_count`, refreshes its `last_reinforced_at`, and the tool output indicates a merge occurred

#### Scenario: Dedup merges similar memory

- **WHEN** the agent calls `memory_add` with content that has >= 0.6 similarity to an existing memory in the same scope
- **THEN** instead of unconditionally merging, `MemoryManager::add_memory()` classifies the relation (Compatible/Contradicts/Ambiguous) and dispatches accordingly: Compatible merges and reinforces (see "Compatible similar memory merges and reinforces"), Contradicts supersedes the existing entry via tombstone (see "Contradicting similar memory supersedes via tombstone"), and Ambiguous merges and flags (see "Ambiguous similar memory merges and flags without LLM"); the previous unconditional-merge behavior is superseded by this relation-based dispatch

#### Scenario: Contradicting similar memory supersedes via tombstone

- **WHEN** the agent calls `memory_add` with content that has >= 0.6 similarity to an existing memory and the relation is classified Contradicts (e.g. existing "auth bug exists", new "auth bug fixed")
- **THEN** the existing memory is marked `superseded_by = <new_id>` (excluded from recall, retained on disk) and the new memory is written as a standalone entry (not merged into the old id)

#### Scenario: Ambiguous similar memory merges and flags without LLM

- **WHEN** the relation is classified Ambiguous
- **THEN** the new content is merged into the existing memory AND the pair is flagged in metadata (or an equivalent pending structure) without invoking an LLM

#### Scenario: Tool returns memory_id on success

- **WHEN** `memory_add` succeeds (new, merged, or superseded)
- **THEN** the tool returns a JSON result containing `success: true`, `memory_id` (the stored entry's UUID appropriate to the outcome), and `merged: boolean` indicating whether content was merged into an existing entry

#### Scenario: Invalid memory_type rejected

- **WHEN** the agent calls `memory_add` with memory_type "InvalidType"
- **THEN** the tool returns an error with code "invalid_memory_type" and does not call `add_memory()`

#### Scenario: Missing content rejected

- **WHEN** the agent calls `memory_add` without the `content` parameter
- **THEN** the tool returns an error with code "missing_content" and does not call `add_memory()`

#### Scenario: Tool registered in daemon registry

- **WHEN** a TUI/daemon session starts and the daemon builds its tool registry
- **THEN** `memory_add` is registered in the daemon tool registry (constructed with the daemon's `MemoryManager`), so the model can call it in TUI/daemon mode

#### Scenario: Tool registered in headless registry

- **WHEN** a headless session starts and builds its tool registry
- **THEN** `memory_add` is registered in the headless tool registry (constructed with the headless `MemoryManager`)

#### Scenario: Tool available to all agents

- **WHEN** any agent (root, explore, plan, or general-purpose subagent) inspects its available tools
- **THEN** `memory_add` is in the agent's tool registry (`filter_allowed_tools` does not exclude it; dedup prevents duplication)

## ADDED Requirements

### Requirement: Effective importance evaluation

The system SHALL compute a memory's **effective importance** as a pure read-time function (no background timer required for the computation itself) combining: the stored base `importance`; a time-decay factor `exp(-ln2 * hours_since(anchor) / type_half_life)` where the anchor is `last_reinforced_at` if set else `timestamp`, and `type_half_life` reuses the existing per-type TTL multipliers relative to the configured age threshold; a hit-rate damping factor `(0.5 + hitrate)` where `hitrate = clamp((hit_count + 1) / (recall_count + 2), 0, 1)` (Laplace-smoothed so a never-recalled memory has hitrate 0.5 and factor 1.0 — neither reward nor penalty); and a staleness multiplier equal to the configured `staleness_penalty` when `stale_marked_at` is Some, otherwise 1.0. A superseded memory (`superseded_by` is Some) SHALL have effective importance 0. Effective importance SHALL be used by recall ranking/filtering, global-memory soft-cap ordering, and consolidation retention (`should_keep`).

#### Scenario: Decay reduces importance over time

- **WHEN** two memories have equal base importance but one was last reinforced 10 half-lives ago and the other recently
- **THEN** the older memory's effective importance is lower than the fresher one's

#### Scenario: Hit-rate damping penalizes recall noise

- **WHEN** a memory has `recall_count=10`, `hit_count=0`, and high base importance
- **THEN** its effective importance is damped below the value it would have with a neutral hit-rate factor

#### Scenario: Never-recalled memory is neutral on hit-rate

- **WHEN** a memory has `recall_count=0`, `hit_count=0`
- **THEN** the hit-rate factor equals 1.0, so it is neither penalized nor rewarded by hit-rate, decaying only by time (and staleness if marked)

#### Scenario: Superseded memory has zero effective importance

- **WHEN** a memory has `superseded_by = Some(id)`
- **THEN** its effective importance is 0 regardless of base importance or age

#### Scenario: Stale-marked memory is downweighted

- **WHEN** a memory has `stale_marked_at = Some(_)` and is not superseded
- **THEN** its effective importance is multiplied by the configured `staleness_penalty` (default 0.5)

### Requirement: Contradiction detection and supersede resolution

When `add_memory()` finds a new memory similar (Jaccard >= 0.6) to an existing same-scope memory, it SHALL classify the relation via a local Tier-1 heuristic only: state-change markers (including but not limited to `fixed`, `resolved`, `removed`, `deprecated`, `migrated`, `no longer`) with high similarity imply Contradicts; clear numeric value drift on a shared key-like token implies Contradicts; subset/same-direction refinement implies Compatible; otherwise Ambiguous. Classification SHALL be conservative (prefer Ambiguous over false Contradicts). Contradicts relations supersede the existing memory by tombstone (`superseded_by` set, excluded from recall, retained on disk) -- the memory is NOT hard-deleted. Compatible relations merge and reinforce. Ambiguous relations merge and flag for possible later resolution without invoking an LLM as part of `add_memory` or `consolidate`.

#### Scenario: State-change marker triggers supersede

- **WHEN** an existing memory "auth bug exists" is present and a new memory "auth bug fixed" is added (Jaccard >= 0.6, contains state-change marker "fixed")
- **THEN** the existing memory is marked `superseded_by` the new id and the new memory is written standalone

#### Scenario: Numeric value drift triggers supersede

- **WHEN** an existing memory references `max_tokens=128000` and a new memory references `max_tokens=4096` (shared tokens, differing numeric token)
- **THEN** the relation is classified Contradicts and the existing memory is superseded via tombstone

#### Scenario: Subset relation is compatible, not contradiction

- **WHEN** an existing memory "use jwt authentication" is present and a new memory "use jwt" is added (subset)
- **THEN** the relation is classified Compatible; the memories merge and the existing memory is reinforced (not superseded)

#### Scenario: Ambiguous pair flagged without LLM

- **WHEN** a similar pair is classified Ambiguous by the Tier-1 heuristic
- **THEN** the new content is merged and the pair is flagged in metadata (or equivalent) with no LLM call

#### Scenario: Superseded memory retained on disk

- **WHEN** a memory is superseded
- **THEN** its JSON file remains on disk (auditable) but it is excluded from recall via effective importance 0

### Requirement: Codebase staleness marking

During `consolidate()` (a local, LLM-free step), when `staleness_check` is enabled, project memories whose content yields one or more extractable filesystem-relative file paths SHALL be checked for path existence on disk. A memory SHALL be marked stale only when **every** extracted path is missing (partial missing MUST NOT mark). The mark is idempotent (`stale_marked_at` set if not already set). Staleness SHALL NOT multiply base `importance` on each run; effective importance applies `staleness_penalty` when the mark is present. `last_reinforced_at` SHALL NOT be refreshed solely because a staleness check ran. Memories with no extractable paths are not stale-marked by this check. This uses the codebase as ground truth -- a signal unique to coding agents.

#### Scenario: All extracted paths missing marks stale once

- **WHEN** `consolidate()` processes a project memory from which one or more file paths are extracted, every extracted path is missing on disk, and the memory is not yet stale-marked
- **THEN** the memory becomes stale-marked and its effective importance is downweighted by `staleness_penalty` without requiring an LLM

#### Scenario: Partial path missing does not mark stale

- **WHEN** `consolidate()` processes a project memory that extracts multiple paths and at least one extracted path still exists
- **THEN** the memory is not stale-marked by that check

#### Scenario: Already-marked stale memory is not stacked

- **WHEN** `consolidate()` processes a memory that already has `stale_marked_at = Some(_)`
- **THEN** consolidate does not apply a further base-importance multiply cycle attributable to staleness (mark remains idempotent)

#### Scenario: Existing-file-only reference unaffected

- **WHEN** `consolidate()` processes a project memory whose extracted paths all still exist
- **THEN** the memory is not stale-marked by that check

#### Scenario: Staleness check is LLM-free

- **WHEN** the codebase-staleness logic runs as part of `consolidate()`
- **THEN** no LLM call is made; the check is a local filesystem existence probe

### Requirement: Recall exploration injection

Recall SHALL honor `exploration_epsilon` (default `0.0`). When the value is `0`, no exploration replacement occurs. When greater than zero, recall SHALL with probability `exploration_epsilon` replace the lowest-ranked injected project memory with a low-effective-importance project memory that is not superseded and has not been recently recalled, to give cold memories a chance to surface. A lightweight recently-recalled set SHALL reduce repeat exploration of the same cold memory within the session.

#### Scenario: Exploration disabled when epsilon is zero

- **WHEN** `exploration_epsilon = 0`
- **THEN** no exploration replacement occurs and recall returns the plain effective-importance top-N

#### Scenario: Exploration replaces lowest-ranked slot when enabled

- **WHEN** `exploration_epsilon > 0`, the exploration draw succeeds, and a suitable cold candidate exists
- **THEN** the lowest-ranked injected project memory is replaced by that candidate
