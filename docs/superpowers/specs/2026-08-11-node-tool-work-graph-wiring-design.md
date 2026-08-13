# Node Tool Static Work-Graph Wiring Design

## Goal

Make the existing node-tool lifecycle enter the static Work-Graph when a node declares compile and test commands. The runtime must execute compile, test, and final verification itself; a model report is not an acceptance signal.

## Scope

Extend the persisted execution-session NodeContract with optional compile_commands and test_commands. BeginNodeTool accepts these optional arrays and stores them with the existing verify_commands and expected_files. VerifyNodeTool delegates to NodeRuntime::run_work_graph when either optional array is configured; otherwise it preserves the established NodeRuntime::verify_node behavior.

## Data Flow

```text
begin_node
  -> persisted NodeContract
  -> verify_node
     -> compile_commands (when non-empty)
     -> test_commands (when non-empty)
     -> verify_commands + changed-file boundary
     -> WorkGraphStep returned as structured tool result
```

The NodeRuntime is the only component that executes commands, persists WorkState fields, and consumes the retry budget. The Tool wrapper exposes the result but does not calculate an edge itself.

## Backward Compatibility

Older session JSON does not contain the two new fields, so both fields use serde defaults to empty vectors. Empty compile and test command lists retain the current verification-only path, including NodeVerifyResult and node status semantics. Existing BeginNodeTool callers remain valid because the schema does not require the new fields.

## Error Handling

Malformed command arrays are rejected by the existing tool-input validation. Runtime command execution errors include the anchor name as context. A failed compile or test produces a structured WorkGraphRunResult with Implement or Escalate; it does not run subsequent anchors. A final boundary violation routes to Escalate.

## Testing

Tests prove:
- a node contract serializes and deserializes the new command arrays;
- a node created through BeginNodeTool persists the arrays;
- VerifyNodeTool runs compile and test before final verification when arrays are present;
- an empty-array legacy contract still uses the previous verification-only path;
- the structured tool output identifies the selected WorkGraphStep.

## Non-goals

- Automatically inferring commands from the project or running after every file write.
- Dynamic graph selection.
- Changing the existing permission model or adding a new LLM agent type.

