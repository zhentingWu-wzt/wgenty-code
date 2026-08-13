# DeepSWE Evaluation Profile Design

## Goal

Improve DeepSWE task completion rates without changing Wgenty Code's normal
interactive or headless-agent behavior. The first delivery is limited to the
Pier driver in `eval/deepswe`.

## Scope

- Detect a task repository's primary ecosystem from a bounded set of manifest
  and test-configuration markers inside the task container.
- Generate an ecosystem-specific evaluation prompt at run time.
- Replace the fixed Vitest fallback guidance with detected, safe test guidance.
- Bias the agent toward implementation, focused failure analysis, and final
  verification rather than mandatory work-graph ceremony.
- Add Python tests for deterministic profile detection and prompt rendering.

The change does not alter the Rust runtime, the general CLI prompt, patch
collection, model selection, or Pier verifier behavior.

## Architecture

`wgenty_code_agent.py` gains a small, pure `ProjectProfile` component. The
async Pier boundary collects seven exact repository markers from `/app` in the
task container, capping `package.json` content at 65,536 bytes. The pure
classifier receives only that in-memory metadata; it never probes the Pier
host filesystem. It returns an immutable description containing an ecosystem
label, package-manager guidance, a preferred test command, and an optional
focused test command template.

The agent driver queries the container metadata before generating settings or
starting `wgenty-code`. A failed probe safely produces the generic profile. It
renders a DeepSWE-specific developer instruction block from the profile and
writes the resulting temporary `settings.json`. The existing base-commit
capture, CodeGraph setup, binary invocation, commit, and `model.patch`
collection remain unchanged.

## Detection Rules

Detection is ordered to avoid ambiguous repositories choosing a generic
command:

1. `Cargo.toml` → Rust: `cargo test`.
2. `go.mod` → Go: `go test ./...`; use `go test ./path -run TestName` when a
   concrete failing test and package are known.
3. `pyproject.toml`, `pytest.ini`, `setup.cfg`, or `tox.ini` → Python:
   `pytest`; use `pytest path::test_name` when known.
4. `package.json` → JavaScript/TypeScript: inspect package scripts for a test
   script and advise `npm test --` as the stable script entry; otherwise use
   the existing package-manager test command only after inspecting the file.
5. No supported marker → generic profile: inspect README, CI, and manifests;
   do not invent a test command.

The implementation only presents commands as guidance. It does not execute a
driver-side automatic full test suite in this phase, since tasks vary widely
in runtime and test topology.

## Evaluation Workflow

The rendered prompt directs the model to:

1. Read task instructions and repository guidance, then spend at most 5–15
   rounds locating the responsible implementation and existing tests.
2. Implement promptly; use a work graph only when the change has independent
   phases or spans multiple subsystems.
3. Run the most focused relevant test after each meaningful implementation
   change.
4. When a test fails, extract its name and assertion, identify the smallest
   responsible behavior, change code rather than expected-behavior tests, and
   rerun that same focused test before broadening validation.
5. Before finishing, run the profile's broad verification command when it is
   known and record its real outcome. The model must not assert tests pass
   without executing them.

## Error Handling

Repository probing treats missing or unreadable files as an unknown profile;
it never fails a trial. Malformed or oversized `package.json` metadata falls
back to generic JavaScript guidance with no command. Prompt generation is pure
and always produces a valid instruction block.

## Testing

Python unit tests create temporary repository layouts and assert:

- Manifest precedence is deterministic.
- Each supported ecosystem receives the expected broad test command.
- Unknown repositories receive no fabricated test command.
- Prompt text uses the selected Rust, Go, Python, and JavaScript focused-test
  template and contains the focused-failure and final-verification rules.
- An async fake Pier execution-surface test proves container metadata is
  collected before settings generation and controls the generated profile.

The tests run with the Python standard library `unittest`, avoiding a new
driver dependency.

## Non-Goals and Follow-up

This phase intentionally does not block patch collection when the agent has
not run a test. After gathering trajectories from a representative DeepSWE
subset, a later runtime-level profile may track test execution and inject a
trusted final-verification gate. That work requires telemetry-backed command
selection and is not part of this driver-focused change.
