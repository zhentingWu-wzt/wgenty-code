---
comet_change: fix-subagent-timeout-default
role: technical-design
canonical_spec: openspec
archived-with: 2026-07-06-fix-subagent-timeout-default
status: final
---

# Subagent Structured Errors & Partial Result Delivery — Technical Design

## Context

This change began as a hotfix to raise the default subagent timeout (`agent.subagent.timeout_secs` 240 → 1800) and remove the hardcoded main-loop `AGENT_LOOP_TIMEOUT`. During verification, the shipped commit (`fa9d8b8`) was found to bundle a second, undocumented capability: a structured `SubagentError` type with partial-result forwarding, spanning `src/teams/subagent_loop.rs`, `src/tools/meta/task.rs`, and `tests/refactor_e2e_test.rs`. Because that capability introduces a new public API and crosses 3+ files, the change was upgraded to the full Comet workflow. This Design Doc retroactively documents the as-built `SubagentError` design (Set B); the timeout tweaks (Set A) are already documented in `openspec/changes/fix-subagent-timeout-default/design.md` and are not re-designed here.

Per the approved brainstorming decision, this is a **document-as-built** design: it records the implementation as the final design and does not propose source changes. Known as-built characteristics are recorded as accepted trade-offs, not action items.

## Problem

Before this change, `run_subagent_loop` returned `Result<String, String>`. On any failure (timeout, budget exhaustion, stuck detection, parse errors, max-rounds), the caller received a bare error string and **none** of the work the subagent had accumulated. This was the original user-reported symptom: a subagent "runs for a while and produces nothing" — the partial work it had done before failing was discarded, and the parent agent could not distinguish failure causes programmatically.

## Design

### Structured error type

`SubagentError` (`src/teams/subagent_loop.rs`) carries three fields:

| Field | Type | Purpose |
|---|---|---|
| `message` | `String` | Human-readable error description, safe to show the LLM |
| `error_type` | `ErrorType` | Categorized failure cause, reused from `src/agent/progress.rs:50` |
| `partial_result` | `Option<String>` | The subagent's last text snapshot before failure — salvageable partial work |

It derives `Debug, Clone` and implements:

- `full_message()` — returns `message`, and when `partial_result` is non-empty, appends a `--- Partial results (before failure) ---` section. This is what the parent agent and the failed transcript see.
- `code()` — maps `ErrorType` to a stable error-code string for `ToolError::code`.
- `Display` — delegates to `full_message()`.
- `From<String>` — bare strings (used by the RLM pipeline path) become `Unknown` with no partial result.

`ErrorType` variants and their `code()` mapping:

| Variant | `code()` | Constructed in `run_subagent_loop`? |
|---|---|---|
| `Timeout` | `subagent_timeout` | yes (overall `tokio::time::timeout`) |
| `BudgetExceeded { limit_k, used }` | `budget_exceeded` | yes (token budget guard) |
| `Stuck { reason }` | `subagent_stuck` | yes (stuck detector **and** max-rounds) |
| `ParseError { message }` | `subagent_parse_error` | yes (consecutive parse errors) |
| `ToolError { tool, message }` | `subagent_tool_error` | **no** — variant exists for the health subsystem; not constructed in the loop |
| `Unknown` | `subagent_error` | yes (`From<String>` / RLM path, line 749) |

### Loop return type

`run_subagent_loop` now returns `Result<String, SubagentError>`. Every terminal error path constructs a `SubagentError` whose `partial_result` is cloned from `text_snapshot: Mutex<Option<String>>`, which is refreshed each iteration when assistant text is processed (`subagent_loop.rs:430`). A small helper `subagent_error(message, error_type, &text_snapshot)` centralizes this for the max-rounds and timeout paths.

### Consumer integration (`TaskTool`, `src/tools/meta/task.rs`)

On `Err(e)`:

1. The failed transcript is saved with `Some(e.full_message())` as its result snapshot — so partial work is preserved in the transcript, not just the in-memory error.
2. The tool returns `ToolError { message: e.full_message(), code: Some(e.code()) }` — the parent agent receives both the partial work (in `message`) and the structured `code`.

The RLM-pipeline path maps its `Result<_, String>` through `.map_err(SubagentError::from)`, so RLM failures surface as `Unknown` / `subagent_error`.

## As-built trade-offs (accepted, no changes in this change)

These are recorded as design characteristics, not defects to fix now:

1. **`partial_result` is delivered inline, not via the mailbox.** Large successful results are offloaded to the JSONL mailbox to bound parent-context tokens (`subagent-result-delivery` spec); large *partial* results on failure are inlined into `ToolError.message` directly. A sufficiently large partial result will consume parent context. Accepted for this change: the primary value is salvaging partial work, which inline delivery satisfies simply.
2. **`From<String>` loses type information.** RLM-pipeline errors degrade to `Unknown` / `subagent_error`; the parent cannot distinguish RLM failure causes by `code`. Accepted: RLM errors are not yet categorized.
3. **`ToolError` variant is dead within the loop.** `subagent_tool_error` is never emitted by `run_subagent_loop`; the variant serves the separate health/stuck subsystem in `progress.rs`. Accepted.
4. **Max-rounds categorized as `Stuck`.** "Exceeded maximum rounds" maps to `ErrorType::Stuck { reason: "exceeded maximum rounds" }` → `subagent_stuck`. A semantic stretch, but `code()` still emits a stable, distinguishable code. Accepted.
5. **`text_snapshot.lock().unwrap()`** will panic if the mutex is poisoned. No other hold site poisons it in practice. Accepted.

## Testing

- Existing coverage (green): 85 subagent-related lib tests, `cargo clippy --lib -- -D warnings` clean, `cargo test --no-run` compiles all targets including the adapted `tests/refactor_e2e_test.rs` (`e.message` access on the new `SubagentError`).
- The delta spec scenarios (below) are assertable via `SubagentError::code()` and `full_message()` behavior. No new tests are added in this change (document-as-built); the scenarios serve as future verification anchors.

## Spec Patch

A delta spec is added to `openspec/changes/fix-subagent-timeout-default/specs/subagent-result-delivery/spec.md` under `## ADDED Requirements`, covering failure-side structured error codes and partial-result delivery. The existing `subagent-result-delivery` spec covers only successful large results; this patch extends the capability to the failure case without modifying existing requirements.
