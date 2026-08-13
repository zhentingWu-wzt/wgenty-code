# Rust Verification Profile Design

## Goal

Ensure every new node created in a Rust project enters the static Compile →
Test → Verify Work-Graph with code-owned external anchors. A model must not be
able to opt out of compilation or testing by omitting command arrays.

## Scope

This change introduces project-local verification-profile detection in the
execution-session runtime. When the session project root contains `Cargo.toml`,
the runtime resolves the node to the `rust` profile and persists the resolved
commands in its `NodeContract`:

1. Compile anchor: `cargo check`
2. Test anchor: `cargo test --all`
3. Final verification anchor: `cargo clippy --all-targets -- -D warnings`

Agent-supplied `verify_commands` remain supported as supplemental final
verification commands. The profile command runs first and exact duplicates are
removed while preserving order. Direct callers of `NodeRuntime` receive the
same enforcement as callers of `BeginNodeTool`.

## Data Model

`VerificationProfile` is a serde-compatible enum stored in `NodeContract` as
the **resolved** profile, not as an untrusted requested mode:

- `none` — no detected managed profile; existing caller-provided anchors are
  retained.
- `rust` — the project root contains `Cargo.toml`; compile, test, and final
  verification commands include the required Rust anchors.

The field has `#[serde(default)]`, resolving absent values to `none` so
existing persisted session JSON retains its established legacy behavior.

Profile resolution happens while `NodeRuntime::begin_node_with_anchors` holds
the coordinator lock long enough to copy the project root. Command detection
and merge are deterministic local operations. No lock is held during command
execution.

## Command Ownership and Tool Contract

`BeginNodeTool` no longer advertises `compile_commands` or `test_commands` as
model-selected inputs. Its required input remains the goal and supplemental
`verify_commands`; the runtime owns the compile and test phases. Existing
programmatic `begin_node_with_anchors` callers remain compatible, but their
arrays can only add anchors after required profile commands and cannot remove
them.

For a non-Rust project, the old contract is intentionally retained: optional
caller-supplied compile/test commands select the existing static graph, and two
empty arrays retain the legacy verification-only path. This change establishes
the profile mechanism without guessing unsafe commands for other ecosystems.

## Routing and Persistence

The profile is resolved before the node is added to the coordinator, then the
fully resolved commands and profile label are persisted in `session.json`.
`verify_current_node` continues to route only from persisted contract fields.
Therefore a resumed session runs the exact profile selected at node creation;
it does not re-detect a possibly changed workspace.

The existing CompileResult, TestResult, VerifyResult, checkpoint writes,
budget accounting, and code-owned `WorkGraphStep` routing are unchanged. The
new profile label makes future audit and dynamic graph policy decisions
traceable without letting an agent alter the selected graph.

## Error Handling

Profile detection only checks for the manifest path and has no fallible IO.
The existing command executor remains the source of truth for anchor outcomes:
missing Cargo, a failed check, or a failed test becomes structured external
anchor state and routes by `WorkGraphStep`. It is never interpreted from model
text.

## Compatibility

Old nodes whose serialized contracts lack `verification_profile` deserialize
to `none` and keep the original behavior. New nodes in non-Rust workspaces also
remain compatible. New Rust nodes always contain non-empty compile and test
arrays, so they cannot take the legacy verification-only path.

## Testing

Focused tests prove that:

- Rust detection returns the code-owned command set;
- a Rust node created with empty or adversarial caller arrays persists the
  mandatory anchors and `rust` profile;
- supplemental final commands are appended without duplicate profile commands;
- non-Rust and legacy serialized contracts preserve their prior behavior;
- VerifyNodeTool executes the resolved Rust compile, test, and final anchors
  in order through the command executor.

## Non-goals

- Inferring profiles for JavaScript, Python, Go, or arbitrary repositories.
- Letting the model choose a profile or replace a required anchor.
- Dynamic graph construction or additional sub-agent roles. Those follow after
  the static entry path and audit data are stable.
