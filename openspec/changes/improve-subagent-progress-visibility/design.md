## Context

The current subagent system has three pain points:

1. **Black-box subagents**: Users cannot perceive what the LLM inside a subagent is doing. The TUI shows `current_tool: "file_read"` but not what file, what the result was, or what the model did next. The subagent's reasoning loop (model thinks → calls tool → sees result → thinks again) is completely invisible. Users want to see: what tool was called → what it returned → what the model is doing now.

2. **Progress opacity**: The TUI shows a static "Subagent running…" label in the status bar regardless of how many subagents are active, their individual progress, or their token consumption. The overlay panel (`SubagentPanel`) shows a tree of nodes with round/tool info but lacks timing, token data, and the action history.

3. **Overly aggressive routing**: The `is_complex_task()` function in `src/tools/meta/task.rs` uses a naive keyword-counting heuristic. Its keyword list includes common English words like "create", "system", "first", "after", "then" — a simple prompt like "create a file called config.json" matches ≥4 keywords and gets routed through the full RLM pipeline (Planner → Decompose → Parallel Execute → Aggregate), adding 3-5 extra LLM round-trips for a task that should take 1.

The daemon progress store is also global (`HashMap<String, SubagentProgress>`), meaning concurrent sessions could cross-contaminate progress data.

### Current Data Flow

```
subagent_loop.rs → emit(SubagentProgress) → make_progress_callback
    → DaemonState.subagent_progress (global HashMap)
    → TUI poll_subagent_progress() every 500ms
    → AppEvent::SubagentUpdate → subagent_tree.upsert()
    → render: status bar (static label) + subagent_panel (tree) + inline card (tree)
```

Key constraints:
- The TUI communicates with the daemon over HTTP (not in-process)
- Subagent execution happens inside tool handlers, which can be sequential or parallel
- The `SubagentProgress` type is in `agent/progress.rs` (standalone, no TUI dependency)
- Progress callbacks are `Arc<dyn Fn(SubagentProgress) + Send + Sync>`

## Goals / Non-Goals

**Goals:**
- Display subagent LLM tool calls with key parameters and tool results (success/failure + summary) in the TUI
- Show a short action history per subagent node so users can trace the model's reasoning loop
- Show live subagent counts in the status bar (active/completed/failed)
- Populate `token_count` in `SubagentProgress.metadata` from actual API usage
- Display per-node elapsed time, round progress, and token usage in the overlay panel
- Refine `is_complex_task()` to reduce false positives — simple single-step tasks should NOT go through RLM
- Log routing decisions so users understand why a task was delegated
- Isolate daemon progress store by session ID

**Non-Goals:**
- Real-time character-by-character streaming of subagent text output
- Changing the RLM pipeline's decomposition algorithm itself
- Adding subagent cancellation/interruption from the TUI
- Persisting subagent progress across daemon restarts
- Modifying the `AgentsService` (old, disconnected agent system) — out of scope
- Making `subagent_history` browsable in the TUI (deferred to future change)

## Decisions

### Decision 1: Action Visibility via Tool Call Log + Model Text

**Choice**: Add a `SubagentAction` struct to `agent/progress.rs` and an `action_log: Vec<SubagentAction>` field to `SubagentProgress`. The subagent loop emits progress events when tools start (with tool name + key params summary). Separately, the model's text responses are captured as `text_snapshot` after each round. Together, the action log shows "what tools the model called" and the text snapshots show "what the model said/thought" — the call→think→call→think loop.

```rust
pub struct SubagentAction {
    pub tool_name: String,           // e.g., "file_read"
    pub params_summary: String,      // e.g., "src/auth.rs"
    pub timestamp_ms: i64,
}
```

The action log is capped at 10 entries per node (newest first), trimmed in the event emitter. The TUI panel shows up to 3 recent actions per node to keep the display compact. **Tool results are intentionally excluded** — they are verbose and noisy; the model's own text response (captured in `text_snapshot`) naturally reflects what it learned from the tool result.

Display order in the panel: latest text snapshot (what the model just said) → then recent tool calls (what actions it took). This matches the model's own loop: think → call tool → think → call tool.

**Alternatives considered**:
- Store tool results → Rejected: too noisy, model text already reflects what it learned
- Separate "action stream" endpoint → Overengineered for a progress visibility feature
- Only show current tool with no history → Doesn't give enough context
- Stream every text chunk → Would flood the event channel; periodic snapshots are sufficient

**Rationale**: The action log + text snapshot combination gives users the essential signal — what the model is doing and what it's thinking — without the noise of raw tool outputs. A small bounded log (10 entries) keeps event payloads manageable.

### Decision 2: Parameter Summarization

**Choice**: Generate a human-readable params summary by extracting the 1-2 most meaningful parameter values from the tool call JSON. For example:
- `file_read` → extract `file_path` → `"src/auth.rs"`
- `grep` → extract `pattern` → `"fn authenticate"`
- `execute_command` → extract `command` (first 80 chars) → `"cargo test --lib"`
- Fallback: first key-value pair from the params object

This is done in the subagent loop when emitting the tool-start progress event.

**Alternatives considered**:
- Show raw JSON params → Too verbose for TUI, hard to scan
- Fixed per-tool display rules → Brittle, requires maintenance as tools change
- Don't summarize, just show tool name → Defeats the purpose of action visibility

**Rationale**: A heuristic-based extraction (first meaningful param) is simple, works for 90% of tools without per-tool configuration, and provides enough context for users to understand what the subagent is doing.

### Decision 3: Status Bar Counters via SubagentTree Aggregation

**Choice**: Add `active_count()`, `completed_count()`, `failed_count()` methods to `SubagentTree` and call them during status bar rendering.

**Alternatives considered**:
- Track counters separately in `App` state → Duplicates data, risks inconsistency
- Emit new `AppEvent` for counter changes → Overcomplicates event handling

**Rationale**: `SubagentTree` already has `count_by_status()`; adding typed accessors keeps the single source of truth and requires no new events.

### Decision 4: Token Counting via API Response Metadata

**Choice**: Capture `usage.input_tokens` and `usage.output_tokens` from each subagent API call's response in `run_subagent_loop()`, accumulate them, and include in the final `Completed` progress event.

**Alternatives considered**:
- Stream tokens during generation → Requires per-chunk progress emissions, too chatty
- Post-hoc calculation → Inaccurate, can't show live counts

**Rationale**: Accumulating per-request and reporting on completion (and optionally every N rounds) balances accuracy with event volume. The `token_count` field already exists in `SubagentMetadata` — we just need to populate it.

### Decision 5: Text Snapshots — Model's "Thinking" Between Tool Calls

**Choice**: Capture the last assistant text response after each subagent round as a `text_snapshot: Option<String>` (max 200 chars) in `SubagentProgress`. Display this prominently beneath each active node in the panel — it represents what the model just said/analyzed/concluded. Combined with the action log (tool calls), this shows the full think→call→think→call loop.

**Rationale**: The text snapshot is the most important signal for understanding what the subagent is doing — it shows the model's reasoning, planning, and analysis. The action log provides context ("it called grep for 'authenticate'"), but the text snapshot shows the model's intent and conclusions ("found 3 auth modules that need updating"). Tool results are excluded because the model's next text response already reflects what it learned from the tool.

### Decision 6: Refined `is_complex_task()` Heuristic

**Choice**: Replace the keyword-counting approach with structural analysis:
1. **Multi-step detection** (high weight): Numbered steps (`\d+\.\s`), explicit sequencing ("first…then…finally")
2. **File references** (medium weight): File paths in backticks or quotes, glob patterns
3. **Dependency declarations** (medium weight): "depends on", "after X completes", "requires Y first"
4. **Length** (low weight): Only trigger at >1000 chars and only as a secondary signal

Remove the current keyword list entirely.

**Alternatives considered**:
- Use an LLM call to classify complexity → Adds latency to the routing decision itself
- Pure length-based threshold → Too crude, long prompts can be simple
- Keep current approach but trim keyword list → Band-aid, doesn't fix root cause

**Rationale**: Structural analysis is fast (regex-based, no API call), more accurate than keyword counting, and easy to tune. It targets the root cause: the current heuristic matches common English words.

### Decision 7: Session-Scoped Progress Store

**Choice**: Change `DaemonState.subagent_progress` from `HashMap<String, SubagentProgress>` to `HashMap<String, HashMap<String, SubagentProgress>>` where the outer key is `session_id`. The progress endpoint accepts `?session_id=<id>` and returns only that session's progress.

**Alternatives considered**:
- Namespace node IDs with session prefix → Leaky, requires parsing
- Keep global but add session field to `SubagentProgress` → Filtering is slower

**Rationale**: Session-scoped storage is the standard pattern; the daemon already has session-scoped state for other entities. The change is mechanical and backward-compatible (existing callers just need to pass `session_id`).

### Decision 8: Routing Rationale Logging

**Choice**: Add a `routing_reason: Option<String>` field to the tool result metadata returned by the `task` tool. The TUI renders this as a dimmed line beneath the "Subagent" tool label (e.g., "routed to RLM: multi-step refactor with 5 files").

**Alternatives considered**:
- Log to stderr only → Not visible in TUI
- New status bar section → Too prominent for diagnostic info

**Rationale**: Tool result metadata is already rendered in the chat area; adding a routing reason field there keeps the information visible but unobtrusive.

## Risks / Trade-offs

- **Action log memory**: Each `SubagentAction` is ~150 bytes; 10 entries × 10 subagents = ~15KB, well within acceptable limits. → Minimal risk.
- **Text snapshot staleness**: The snapshot updates only between rounds, so during a long tool execution it may feel "stale." → Mitigation: Show "executing <tool>…" indicator alongside the last snapshot to make clear the model is waiting for a tool result.
- **Parameter summarization accuracy**: The heuristic-based extraction may produce unhelpful summaries for some tools. → Mitigation: Fallback to showing the tool name without params; iterate on extraction rules per tool.
- **Heuristic tuning**: The new `is_complex_task()` may have its own false positive/negative patterns. → Mitigation: Add the routing reason to tool output so users can see and report misclassifications; iterate on patterns.
- **Status bar width**: Adding subagent counters to the status bar risks overflow on narrow terminals. → Mitigation: Truncate gracefully; the status bar already handles overflow with ellipsis.
- **Session ID plumbing**: Passing `session_id` through the progress pipeline touches multiple layers. → Mitigation: The session ID is already available in both the daemon handler context and the TUI's `DaemonClient`; the change is mechanical.
- **API response parsing**: Token counts may not be available from all providers in the same format. → Mitigation: Use the provider-agnostic `ApiClient` response wrapper; default to `None` if unavailable.
