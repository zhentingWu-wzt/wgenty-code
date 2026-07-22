## MODIFIED Requirements

### Requirement: Memory storage via MemoryManager

All memories SHALL be stored exclusively via `MemoryManager`, using its per-file Storage backend. Memories SHALL be physically separated by scope:
- **Project memories** SHALL be stored at `<project_root>/.wgenty-code/memory/<id>.json`
- **Global memories** SHALL be stored at `~/.wgenty-code/memory/<id>.json`

`project_root` SHALL equal the current working directory (CWD), with no upward search for project markers. Each memory SHALL use the `context::MemoryEntry` type with fields: id, memory_type, content, timestamp, importance, tags, metadata, AND the feedback-tracking fields `recall_count`, `hit_count`, `last_reinforced_at` (Option, None meaning use `timestamp` as the decay anchor), and `superseded_by` (Option, the id of a memory that supersedes this one). The four feedback fields SHALL deserialize with defaults (`recall_count=0`, `hit_count=0`, `last_reinforced_at=None`, `superseded_by=None`) when absent, so existing memory JSON files load without migration. The memory file's filename SHALL remain the stable `id` (UUID); semantic display slugs SHALL NOT replace the id-as-filename for semantic memories (the id is load-bearing, referenced by `superseded_by`, the TF-IDF index, merge-keep-id, and import dedup). `MemoryManager` SHALL track each loaded memory's origin (Project or Global) and persist memories to the directory matching their scope.

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

- **WHEN** a memory JSON file written before this change (lacking `recall_count`/`hit_count`/`last_reinforced_at`/`superseded_by`) is loaded
- **THEN** it deserializes successfully with `recall_count=0`, `hit_count=0`, `last_reinforced_at=None` (decay anchored at the original `timestamp`), and `superseded_by=None`

### Requirement: Memory recall at session startup

At session startup, `MemoryManager::load()` SHALL load project memories from `<CWD>/.wgenty-code/memory/` and global memories from `~/.wgenty-code/memory/`. `MemoryManager::search_memories(query)` SHALL retrieve only project memories matching the query via the TF-IDF index (global memories are not indexed and are injected verbatim every turn). Recall ranking, threshold filtering, and global-memory soft-cap ordering SHALL use **effective importance** (see "Effective importance evaluation"). A superseded memory (`superseded_by` is Some) SHALL be excluded from recall. Recall scoring SHALL be multi-cue: `score = α·tfidf + β·symbol_overlap + γ·recency`, where `symbol_overlap` measures overlap between the current task's symbol context (open files, function being edited, stack frames) and the memory's content (see "Symbol-aware multi-cue recall"). Recall SHALL, with probability `exploration_epsilon` (default 0.15), replace the lowest-ranked injected project memory with a low-effective-importance project memory not recently recalled (see "Recall exploration injection"). When a recalled memory is an episodic entry exceeding a length threshold, it SHALL be restated against the current query before injection (see "Recall-time reconstruction"); short semantic facts SHALL be injected verbatim to avoid hot-path LLM cost.

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

#### Scenario: Symbol overlap boosts relevant memories

- **WHEN** the current task is editing `verify_token` in `auth/jwt.rs` and two memories have equal TF-IDF score but only one mentions `verify_token`/`jwt`
- **THEN** the symbol-overlapping memory ranks higher due to the `β·symbol_overlap` term

#### Scenario: Episodic entry restated before injection

- **WHEN** a recalled memory is an episodic entry whose content exceeds the restate length threshold
- **THEN** it is passed through an LLM restate pass that extracts the slice relevant to the current query before being injected (short semantic facts are injected verbatim, incurring no extra LLM call)

#### Scenario: Exploration injects a cold memory

- **WHEN** recall produces a top-N result and the exploration draw (probability `exploration_epsilon`) succeeds
- **THEN** the lowest-ranked injected project memory is replaced by a low-effective-importance project memory not recently recalled

### Requirement: Time-gated memory consolidation

`AutoDreamService::check_and_run()` SHALL be called at session startup before recall, in both TUI/daemon and headless modes. The gate thresholds SHALL be `min_hours = 1` and `min_sessions = 1`. The session-scan throttle SHALL be 10 minutes.

AutoDream SHALL NOT maintain its own disk-based consolidation lock. Cross-process mutual exclusion SHALL be provided solely by `MemoryManager::consolidate()`'s internal `ConsolidationFileLock` (at `~/.wgenty-code/memory/.consolidation.lock`). AutoDream's in-memory `is_consolidating` flag SHALL be reset on each `check_and_run` invocation.

`MemoryManager::consolidate()` does not invoke any LLM call -- it is pure local computation (TF-IDF similarity merge, TTL decay, orphan-file reconcile, index rebuild, codebase-staleness decay). This is the premise that permits the aggressive 1h/1session gate. The retention decision (`should_keep`) SHALL use **effective importance** instead of raw `importance`. `consolidate()` SHALL additionally apply a codebase-staleness decay to project memories referencing non-existent file paths. ALL LLM-assisted operations -- ambiguous-pair supersede resolution AND episodic replay extraction -- SHALL occur in SEPARATE post-consolidation steps invoked by `dream` AFTER `consolidate()`, NOT inside `consolidate()` (see "Contradiction detection and supersede resolution" and "Offline replay consolidation").

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

- **WHEN** AutoDream triggers `consolidate()` while a concurrent `memory dream` invocation already hold the `ConsolidationFileLock`
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
- **THEN** no LLM call is made; consolidation completes via local TF-IDF merge, TTL decay (using effective importance), orphan reconcile, codebase-staleness decay, and index rebuild. Any LLM supersede resolution and episodic replay extraction run as separate post-consolidation steps and are NOT part of `consolidate()`.

#### Scenario: Stale-path memory decayed during consolidation

- **WHEN** `consolidate()` processes a project memory whose content references a file path that no longer exists on disk
- **THEN** that memory's `importance` is reduced (multiplied by a penalty factor) and `last_reinforced_at` is not refreshed

### Requirement: Proactive memory capture via tool

The system SHALL provide a `memory_add` tool that allows the agent to proactively write a memory entry at any point during a conversation, without waiting for context compaction. The tool SHALL accept parameters: `content` (required string), `memory_type` (enum: Knowledge/Preference/Session/Conversation/Task/Error/Insight/Decision), `scope` (project|global, default project), and optional `importance`. When the new memory is similar (Jaccard >= 0.6) to an existing memory in the same scope, `MemoryManager::add_memory()` SHALL classify the relation as Compatible, Contradicts, or Ambiguous (see "Contradiction detection and supersede resolution") instead of unconditionally merging: Compatible relations merge AND reinforce the existing memory; Contradicts relations supersede the existing memory (tombstone, not hard delete); Ambiguous relations merge AND flag the pair for later LLM resolution.

#### Scenario: Agent proactively writes a project memory

- **WHEN** the agent calls `memory_add` with content "note_edit tool uses NoteStore but is registered with store:None, so it doesn't persist", memory_type "Knowledge", scope "project"
- **THEN** `MemoryManager::add_memory()` is called with a `MemoryEntry` of type Knowledge and `MemoryOrigin::Project`, and the memory is saved to `<CWD>/.wgenty-code/memory/<id>.json`

#### Scenario: Agent proactively writes a global memory

- **WHEN** the agent calls `memory_add` with content "Always read actual settings.json before assuming config defaults", scope "global"
- **THEN** `MemoryManager::add_memory()` is called with `MemoryOrigin::Global`, and the memory is saved to `~/.wgenty-code/memory/<id>.json`

#### Scenario: Compatible similar memory merges and reinforces

- **WHEN** the agent calls `memory_add` with content that has >= 0.6 similarity to an existing memory and the relation is classified Compatible
- **THEN** `MemoryManager::add_memory()` merges the new content into the existing memory entry, increments the existing memory's `hit_count`, refreshes its `last_reinforced_at`, and the tool output indicates a merge occurred

#### Scenario: Contradicting similar memory supersedes via tombstone

- **WHEN** the agent calls `memory_add` with content that has >= 0.6 similarity to an existing memory and the relation is classified Contradicts (e.g. existing "auth bug exists", new "auth bug fixed")
- **THEN** the existing memory is marked `superseded_by = <new_id>` (excluded from recall, retained on disk), its `importance` is reduced by the supersede penalty, and the new memory is written as a standalone entry (not merged)

#### Scenario: Ambiguous similar memory merges and flags for review

- **WHEN** the relation is classified Ambiguous
- **THEN** the new content is merged into the existing memory AND the pair is flagged for later LLM supersede resolution during the next `dream`

#### Scenario: Tool returns memory_id on success

- **WHEN** `memory_add` succeeds (new, merged, or superseded)
- **THEN** the tool returns a JSON result containing `success: true`, `memory_id` (the stored entry's UUID), and `merged: boolean` indicating whether it was merged into an existing entry

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

The system SHALL compute a memory's **effective importance** as a pure read-time function (no background timer, no disk write) combining: the stored base `importance`; a time-decay factor `exp(-ln2 * hours_since(last_reinforced_at) / type_half_life)` where the anchor is `last_reinforced_at` if set else `timestamp`, and `type_half_life` reuses the existing per-type TTL multipliers; and a hit-rate damping factor `(0.5 + 0.5 * hitrate)` where `hitrate = (hit_count + 1) / (recall_count + 2)` (Laplace-smoothed, so a never-recalled memory is neutral at 0.5 and a frequently-recalled-but-never-engaged memory is damped toward 0). A superseded memory (`superseded_by` is Some) SHALL have effective importance 0. Effective importance SHALL be used by recall ranking/filtering, global-memory soft-cap ordering, and consolidation retention (`should_keep`).

#### Scenario: Decay reduces importance over time

- **WHEN** two memories have equal base importance but one was last reinforced 10 hours ago and the other 1 hour ago
- **THEN** the older memory's effective importance is lower than the fresher one's

#### Scenario: Hit-rate damping penalizes recall noise

- **WHEN** a memory has `recall_count=10`, `hit_count=0`, and high base importance not yet TTL-expired
- **THEN** its effective importance is damped below its base importance by the hit-rate factor

#### Scenario: Never-recalled memory is neutral

- **WHEN** a memory has `recall_count=0`, `hit_count=0`
- **THEN** the hit-rate factor equals 1.0, so it is neither penalized nor rewarded, decaying only by time

#### Scenario: Superseded memory has zero effective importance

- **WHEN** a memory has `superseded_by = Some(id)`
- **THEN** its effective importance is 0 regardless of base importance or age

### Requirement: Engagement attribution and reinforcement

The system SHALL maintain a per-session recall attribution window that records, for each turn, the ids and distinctive (high-IDF) tokens of project memories injected via recall. On each subsequent user message, the window SHALL be settled: for each pending injected memory, if its distinctive tokens appear in the user message, the memory is reinforced (`hit_count++`, `last_reinforced_at` refreshed) and removed from the window. Reinforcement credit SHALL be weighted by recency decay `exp(-turn_delta / decay_tau_turns)` (default 2.0) so delayed engagement partially credits and very stale entries self-expire. A topic boundary -- detected when lexical overlap between consecutive user messages drops below 0.15 -- SHALL close all open attribution windows. Only user-side engagement SHALL drive positive reinforcement in v1. Reinforcement increments `recall_count` whenever a memory is injected into the window.

#### Scenario: Immediate engagement reinforces memory

- **WHEN** a recalled memory's distinctive high-IDF tokens appear in the next user message
- **THEN** that memory's `hit_count` is incremented and `last_reinforced_at` refreshed

#### Scenario: Delayed engagement partially credits

- **WHEN** a recalled memory's distinctive tokens appear in the user message two turns after injection
- **THEN** the memory is reinforced but with credit weighted by the recency decay factor (less than immediate credit)

#### Scenario: Stale unengaged entry self-expires

- **WHEN** a window entry's recency weight drops below 0.05 without any engagement
- **THEN** it is dropped from the window with no reinforcement (its `recall_count` was already incremented at injection)

#### Scenario: Topic boundary closes the window

- **WHEN** lexical overlap between the current and previous user messages drops below 0.15
- **THEN** all pending attribution entries are closed without reinforcement

#### Scenario: Common tokens do not trigger engagement

- **WHEN** the user message shares only low-IDF common tokens with an injected memory
- **THEN** no reinforcement occurs (engagement requires at least one high-IDF distinctive token)

### Requirement: Contradiction detection and supersede resolution

When `add_memory()` finds a new memory similar (Jaccard >= 0.6) to an existing same-scope memory, it SHALL classify the relation via a local Tier-1 heuristic: state-change markers (`fixed`, `resolved`, `removed`, `deprecated`, `migrated`, `no longer`) with high similarity imply Contradicts; numeric value drift implies Contradicts; subset relation implies Compatible; otherwise Ambiguous. Contradicts relations supersede the existing memory by tombstone (`superseded_by` set, excluded from recall, retained on disk) and reduce its importance by the supersede penalty (default 0.3) -- the memory is NOT hard-deleted. Ambiguous pairs SHALL be flagged for later resolution. A SEPARATE post-consolidation step (invoked by `dream` after `consolidate()`, NOT inside it) SHALL batch all flagged ambiguous pairs into a single LLM call classifying each as supersede/merge/both; this step SHALL invoke an LLM only when at least one flagged pair exists. `consolidate()` itself SHALL remain LLM-free.

#### Scenario: State-change marker triggers supersede

- **WHEN** an existing memory "auth bug exists" is present and a new memory "auth bug fixed" is added (Jaccard >= 0.6, contains state-change marker "fixed")
- **THEN** the existing memory is marked `superseded_by` the new id, its importance is reduced, and the new memory is written standalone

#### Scenario: Numeric value drift triggers supersede

- **WHEN** an existing memory references `max_tokens=128000` and a new memory references `max_tokens=4096` (shared tokens, differing numeric token)
- **THEN** the relation is classified Contradicts and the existing memory is superseded via tombstone

#### Scenario: Subset relation is compatible, not contradiction

- **WHEN** an existing memory "use jwt authentication" is present and a new memory "use jwt" is added (subset)
- **THEN** the relation is classified Compatible; the memories merge and the existing memory is reinforced (not superseded)

#### Scenario: Ambiguous pair flagged for LLM resolution

- **WHEN** a similar pair is classified Ambiguous by the Tier-1 heuristic
- **THEN** the new content is merged and the pair is flagged for the next `dream`'s post-consolidation LLM step

#### Scenario: Dream LLM batch resolution runs separately from consolidate

- **WHEN** `dream` runs and flagged ambiguous pairs exist
- **THEN** `consolidate()` completes first (LLM-free), then a separate step makes one batched LLM call classifying each pair as supersede/merge/both and applies the results

#### Scenario: No ambiguous pairs means no LLM call

- **WHEN** `dream` runs and no pairs are flagged ambiguous
- **THEN** the post-consolidation supersede-resolution step is a no-op and no LLM call is made

#### Scenario: Superseded memory retained on disk

- **WHEN** a memory is superseded
- **THEN** its JSON file remains on disk (auditable) but it is excluded from recall via effective importance 0

### Requirement: Codebase staleness decay

During `consolidate()` (a local, LLM-free step), project memories whose content references a file path (e.g. `src/...rs:N`) SHALL be checked for path existence on disk. A memory referencing a path that no longer exists SHALL have its `importance` reduced by a staleness penalty (default 0.5x) and its `last_reinforced_at` SHALL NOT be refreshed. This uses the codebase as ground truth -- a signal unique to coding agents.

#### Scenario: Deleted-file reference decays

- **WHEN** `consolidate()` processes a project memory referencing `src/old_module.rs:42` and that file no longer exists
- **THEN** the memory's `importance` is halved and `last_reinforced_at` is not refreshed

#### Scenario: Existing-file reference unaffected

- **WHEN** `consolidate()` processes a project memory referencing a file path that still exists
- **THEN** the memory's `importance` is not reduced by staleness (other decay/retention rules still apply)

#### Scenario: Staleness check is LLM-free

- **WHEN** the codebase-staleness decay runs as part of `consolidate()`
- **THEN** no LLM call is made; the check is a local filesystem existence probe

### Requirement: Episodic memory store

The system SHALL maintain a separate episodic memory layer at `<project>/.wgenty-code/episodes/`, distinct from the semantic memory store. Episodic entries SHALL be append-mostly: written once per decision point (and/or session-end summary), replayed and pruned, but NOT merged, NOT referenced by `superseded_by`, and NOT indexed by the semantic TF-IDF index. Each episodic file SHALL be named `<YYYYMMDD-HHMM>-<ascii-slug>-<shortid>.json` where the date prefix provides chronological `ls` ordering, the ASCII slug (derived from symbols/keywords, not free-form non-ASCII text) provides cross-platform-safe human readability, and the shortid guarantees uniqueness; the stable id SHALL reside in the JSON content (not the filename), because episodic entries are append-mostly and do not require a stable filename. Episodic content SHALL record what happened (decisions made, files touched, bugs hit, user requests) plus a `pain_score` (see "pain_score salience"). The semantic memory file naming (UUID-as-filename, load-bearing id) SHALL remain unchanged.

#### Scenario: Episodic entry written to episodes directory

- **WHEN** a decision point or session-end summary produces an episodic record
- **THEN** it is saved as `<project>/.wgenty-code/episodes/<YYYYMMDD-HHMM>-<slug>-<shortid>.json` with the stable id in the JSON content

#### Scenario: Episodes listed chronologically via ls

- **WHEN** the episodes directory is listed
- **THEN** files appear in chronological order by virtue of the date-prefixed filename (the filesystem acts as the index, no separate DB)

#### Scenario: Episodic filename is ASCII-safe

- **WHEN** an episodic record's description contains non-ASCII (e.g. Chinese) text
- **THEN** the filename slug is derived from ASCII symbols/keywords (the non-ASCII description lives in the JSON content, not the filename) to remain cross-platform safe

#### Scenario: Semantic memory naming unchanged

- **WHEN** a semantic memory is stored
- **THEN** it is still saved as `<id>.json` (UUID filename, load-bearing id), unaffected by the episodic layer

#### Scenario: Episodic entries excluded from semantic indexing

- **WHEN** the semantic TF-IDF index is built
- **THEN** episodic entries are NOT indexed (they are replayed/pruned, not keyword-recalled like semantic memories)

### Requirement: Offline replay consolidation

`dream` SHALL, after `consolidate()` completes, run a separate `replay_extract()` step that reads recent episodic entries and uses a batched LLM call to: extract durable facts and merge them into the semantic store; deduplicate semantically-equivalent episodes; flag/resolve contradictions against existing semantic memories (via supersede tombstone); and prune low-frequency low-pain episodic entries. This step SHALL invoke an LLM only when recent unaired episodes exist; otherwise it is a no-op. This step is SEPARATE from `consolidate()` and does NOT violate the LLM-free invariant of `consolidate()`. Episodes with higher `pain_score` SHALL receive higher consolidation weight (prioritized transfer to semantic).

#### Scenario: Replay extracts facts into semantic store

- **WHEN** `replay_extract()` runs over recent episodes containing "this project uses pnpm not npm"
- **THEN** a semantic memory is created/merged capturing that fact, and the source episode is eligible for pruning

#### Scenario: Replay deduplicates equivalent episodes

- **WHEN** three episodes describe the same bug fix in different sessions
- **THEN** `replay_extract()` merges them into a single preferred semantic memory rather than creating three duplicates

#### Scenario: Replay resolves episode-vs-semantic contradiction

- **WHEN** a new episode contradicts an existing semantic memory
- **THEN** the contradiction is resolved via supersede tombstone (not deletion), consistent with the contradiction-detection requirement

#### Scenario: Replay prunes low-value episodes

- **WHEN** an episode has low pain_score, low frequency, and no recent access
- **THEN** it is pruned from the episodic store after its durable facts (if any) have been extracted into semantic

#### Scenario: Replay is no-op without unaired episodes

- **WHEN** `replay_extract()` runs and no recent unaired episodes exist
- **THEN** no LLM call is made (the step is a no-op)

#### Scenario: Replay runs after consolidate, separately

- **WHEN** `dream` runs
- **THEN** `consolidate()` (LLM-free) completes first, then `replay_extract()` (LLM) runs as a separate step; `consolidate()` makes no LLM call

### Requirement: Symbol-aware multi-cue recall scoring

Recall SHALL score candidate memories with a multi-cue function `score = α·tfidf + β·symbol_overlap + γ·recency` rather than TF-IDF alone. The `symbol_overlap` term SHALL measure overlap between the current task's symbol context -- open files, the function/symbol being edited, and stack-frame symbols, collected in the agent loop -- and the symbols extracted from each memory's content (via CodeGraph/LSP symbol tables or regex extraction). This leverages coding-agent-unique signal that generic chat agents lack, without requiring embeddings. The weights α/β/γ SHALL be configurable.

#### Scenario: Symbol context collected from agent loop

- **WHEN** a turn is processed
- **THEN** the current task's symbol context (open files, edited symbol, stack frames) is collected and made available to recall scoring

#### Scenario: Symbol overlap augments TF-IDF ranking

- **WHEN** two memories have near-equal TF-IDF scores but different symbol overlap with the current task
- **THEN** the memory with higher symbol overlap ranks higher due to the `β·symbol_overlap` term

#### Scenario: Symbol scoring works without embeddings

- **WHEN** recall runs in a configuration without embeddings
- **THEN** symbol_overlap still augments recall (it derives from CodeGraph/LSP/regex, not embeddings)

### Requirement: pain_score salience

The system SHALL derive a `pain_score` per memory from observable friction signals in the agent loop: `exec_command` failure/retry counts, guardian denials, user corrections (same-intent rephrasing), and `undo` calls. During compaction extraction, the LLM SHALL record `pain_score` into the memory's importance/metadata. Higher-pain memories SHALL receive higher consolidation weight (prioritized transfer to semantic during replay) and slower decay. This is a digital approximation of the brain's emotional-salience weighting, moving importance from a single LLM-assigned scalar toward multi-dimensional salience.

#### Scenario: Friction signals raise pain_score

- **WHEN** a turn experiences repeated `exec_command` failures and a guardian denial before succeeding
- **THEN** the extracted memory for that turn carries a higher `pain_score` than a frictionless turn

#### Scenario: High-pain memory prioritized in replay

- **WHEN** `replay_extract()` selects episodes to consolidate into semantic
- **THEN** higher-pain episodes are prioritized (transferred before low-pain ones)

#### Scenario: pain_score recorded without a new storage backend

- **WHEN** pain_score is computed
- **THEN** it derives from existing agent-loop friction signals (no new storage dependency for v1); once the episodic layer exists, pain is also recorded per episode

### Requirement: Recall-time reconstruction

Recall SHALL treat retrieval as reconstruction, not static retrieval. When a recalled memory is an episodic entry whose content exceeds a length threshold, it SHALL be passed through an LLM restate pass that extracts the slice relevant to the current query before injection (reducing tokens and improving relevance); short semantic facts SHALL be injected verbatim to avoid per-turn hot-path LLM cost. Additionally, when a recalled memory is detected to conflict with the current codebase state (grounding check at read time), the recall SHALL emit a soft "verify me" signal or trigger a write-back update (read-time reconsolidation), extending contradiction detection from write-time to read-time.

#### Scenario: Long episodic entry restated before injection

- **WHEN** a recalled episodic entry exceeds the restate length threshold
- **THEN** an LLM restate pass extracts the query-relevant slice before injection (rather than injecting the full entry verbatim)

#### Scenario: Short semantic fact injected verbatim

- **WHEN** a recalled memory is a short semantic fact below the restate threshold
- **THEN** it is injected verbatim with no extra LLM call (hot-path cost avoided)

#### Scenario: Stale-at-recall triggers verify signal

- **WHEN** a recalled memory references a symbol/file that grounding check finds has changed since the memory was written
- **THEN** recall emits a soft "verify me" signal (or triggers a write-back update), surfacing the staleness rather than injecting a potentially-wrong memory silently

### Requirement: Recall exploration injection

Recall SHALL, with probability `exploration_epsilon` (default 0.15), replace the lowest-ranked injected project memory with a low-effective-importance project memory that has not been recently recalled, to give cold memories a chance to earn reinforcement feedback (mitigating rich-get-richer). A lightweight "recently recalled" set SHALL prevent re-exploring the same cold memory repeatedly. Setting `exploration_epsilon = 0` SHALL disable exploration entirely.

#### Scenario: Exploration replaces lowest-ranked slot

- **WHEN** recall produces a top-N result and the exploration draw succeeds
- **THEN** the lowest-ranked injected project memory is replaced by a low-effective-importance project memory not recently recalled

#### Scenario: Exploration disabled when epsilon is zero

- **WHEN** `exploration_epsilon = 0`
- **THEN** no exploration replacement occurs and recall returns the plain top-N
