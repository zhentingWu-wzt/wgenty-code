## Why

Users currently have no clear visibility into what subagents are doing during execution — the TUI shows only a static "Subagent running…" label regardless of whether 1 or 10 subagents are active. More importantly, users cannot perceive what the LLM inside each subagent is actually doing: what tools it calls, what those tools return, and how the model reacts to results. The subagent is a black box — users see a node labeled "task: refactor module" but have no idea whether it's reading files, searching code, running commands, or stuck in a loop.

Additionally, the `is_complex_task()` heuristic uses overly broad keyword matching that causes trivial single-step tasks to be routed through the expensive RLM pipeline (Planner → Decompose → Parallel Execute → Aggregate), wasting time and tokens on unnecessary orchestration.

## What Changes

### Subagent LLM Action & Response Visibility
- **Tool call visibility**: Display what tool each subagent is currently calling, with key parameters (e.g., `file_read("src/auth.rs")` instead of just `executing: file_read`), so users can see what action the model is taking
- **Model text visibility**: Capture and display the model's text responses between tool calls — what it's analyzing, planning, or concluding — so users perceive the model's "thinking"
- **Action history log**: Each subagent node accumulates a short history of recent tool calls (name + params) and model text, visible in the overlay panel so users can trace the subagent's call→think→call→think loop
- **Current action display**: The inline subagent card shows the current tool with params plus the model's most recent text output, giving a live sense of "what the model just said → what it's doing now"

### Subagent Progress Visibility
- **Status bar counters**: Show active/completed/failed subagent counts in real-time (e.g., "3 active · 5/8 done") instead of a static label
- **Per-node timing**: Display elapsed time and round progress for each subagent node in the tree panel
- **Token tracking**: Populate `SubagentProgress.metadata.token_count` with actual API usage data per subagent

### Task Routing Improvements
- **Refined complexity detection**: Replace the current naive keyword-counting heuristic (which flags common words like "create", "system", "first") with structural analysis that considers:
  - Actual multi-step structure (numbered steps, file path references, explicit dependencies)
  - Prompt length as a secondary signal rather than primary trigger
- **Transparent routing decisions**: Log why a task was routed to RLM vs. direct subagent vs. inline execution, visible in the TUI

### Progress Store Fixes
- **Per-session isolation**: Scope daemon progress storage by session ID so concurrent sessions don't cross-contaminate progress data

## Capabilities

### New Capabilities
- `subagent-action-visibility`: Display the LLM's tool calls (with key params) and tool results (success/failure + summary) for each subagent, so users can perceive the model's action→response loop
- `subagent-status-display`: Real-time subagent progress counters in status bar, per-node elapsed time, round progress, and token consumption visible in both the inline card and overlay panel
- `task-complexity-detection`: Refined heuristics for deciding whether a task requires RLM pipeline decomposition vs. direct subagent execution vs. inline tool execution, with transparent routing rationale logged to TUI

### Modified Capabilities
<!-- No existing specs to modify -->

## Impact

- **`src/agent/progress.rs`**: Add `SubagentAction` struct (tool_name, params_summary, timestamp) and `action_log: Vec<SubagentAction>` + `current_params: Option<String>` fields to `SubagentProgress`; populate `token_count` in `SubagentMetadata`
- **`src/teams/subagent_loop.rs`**: Emit action events when tools start and complete; capture tool call parameters and result summaries; accumulate action log
- **`src/tools/meta/task.rs`**: Rewrite `is_complex_task()` with structural analysis; add routing rationale logging; pass action context through progress callbacks
- **`src/tools/meta/rlm/pipeline.rs`**: Propagate action events from RLM subagents through the aggregated progress callback
- **`src/tui/components/status.rs`**: Replace static "Subagent running…" with live counter display
- **`src/tui/components/subagent_panel.rs`**: Render action log (tool calls + results) beneath each subagent node; add per-node elapsed time, token count
- **`src/tui/components/chat.rs`**: Update inline subagent card to show current tool with params and last result
- **`src/tui/components/subagent_tree.rs`**: Add aggregation methods for status bar counters
- **`src/tui/agent/core.rs`**: Track subagent API token usage and pass to progress callbacks
- **`src/tui/agent/tool_dispatch.rs`**: Same token tracking for parallel execution path
- **`src/daemon/state.rs`**: Change progress store from global `HashMap` to `HashMap<SessionId, HashMap<NodeId, SubagentProgress>>`
- **`src/daemon/handlers.rs`**: Accept session ID param in progress endpoint; filter by session
- **`src/tui/client.rs`**: Pass session ID in `poll_subagent_progress()` requests
