# Static Work-Graph v1 Design

## Goal

Turn the existing verification-only pilot into a small, deterministic software-engineering work graph. The graph must use externally observed command results and coordinator-owned budget state to route work; an agent's natural-language completion claim is never a success signal.

## Scope

This change implements one fixed graph for a verifiable implementation node:

```text
Implement -> CompileAnchor -> TestAnchor -> VerifyGate
                                      |            |
                                      +--- fail ---+
                                                   v
                              retry when budget remains -> Implement
                              otherwise             -> Escalate
```

`CompileAnchor`, `TestAnchor`, and `VerifyGate` execute configured commands through the existing execution-session verification infrastructure. Their exit status, stderr, and discovered failing cases are the external anchors. `Implement` remains the existing general-purpose agent work; this change does not add new LLM node types or dynamic graph generation.

## Architecture

`WorkState` remains the per-task shared state. The implementation adds real production write paths for `generated_diff`, `compile_result`, `test_result`, and coordinator-owned budget consumption. The coordinator, rather than an agent, records the diff from the workspace and updates the budget after each failed verification.

A pure routing function consumes only `WorkState` and returns a typed next step. `BoundaryViolation` routes directly to escalation. A compile or test command failure routes to retry only while the coordinator-owned iteration budget remains. All successful anchors route to completion. No routing decision parses formatted model output.

The verification node is a veto gate: it cannot spawn children or mutate the workspace. It receives a clean verification context consisting of the commands, expected files, and anchored work-state inputs.

## Alternatives Considered

1. Add specialised LLM compile/test/review agents. Rejected: deterministic commands provide the relevant evidence with less cost and less self-review risk.
2. Let the primary model select the next graph edge. Rejected: routing must be reproducible, auditable, and budget-enforced.
3. Generate a task-specific graph dynamically. Deferred: the existing node contracts and this fixed graph must first demonstrate reliable evaluation results.

## Error Handling and Recovery

Every anchor result is persisted with the current turn's `WorkState` checkpoint. On restart, the most recent state determines the next route without reinterpreting chat history. A failed command records its exit code and stderr. A failed boundary check records unexpected files. No budget remaining, a boundary violation, or an unclassifiable verification failure ends in `Escalate` rather than another automatic modification attempt.

## Testing

Tests cover each route: all anchors pass; compile fails with budget remaining; test fails after budget exhaustion; and boundary violation escalates immediately. Tests also prove that the state saved to a checkpoint restores the corresponding structured anchor result. Existing execution-session integration tests continue to protect rollback and retry behavior.

## Non-goals

- Dynamic Work-Graph generation or GoA-style edge weighting.
- New LLM node types for compilation, testing, or review.
- Replacing session state, checkpoint storage, or existing permission policy.
- Automatic final approval in place of the existing human permission model.
