# Work-Graph Org-Graph Binding Design

## Goal

Bind every selected Work-Graph role to an existing `NodeRegistry` contract and persist the identity of any dispatched child Agent as the execution instance of that role.

## Scope

- Do not add new node types.
- Resolve and validate `WorkGraphPlan` roles against `NodeRegistry` at selection time.
- Persist a role-to-contract summary with the selected plan.
- Persist RootCause child identity in graph audit state.
- Keep existing external-anchor, permission, checkpoint, and recovery behavior.

## Design

`NodeRegistry` remains the Org-Graph source of truth. The selector receives an immutable registry, rejects missing roles, and produces a plan whose nodes contain the registered contract name and role. Runtime dispatch records the selected role (`RootCause`) and child Agent ID in a durable audit event. The child remains an Agent lifecycle record; the graph event provides the role binding and does not duplicate the child as an `exec_session::Node`.

Legacy checkpoints without binding metadata remain readable. New plans are validated before persistence. A dispatch is accepted only when the selected plan contains the required role and edge.

## Verification

- Unit tests reject plans referencing unregistered roles.
- Unit tests round-trip role binding metadata.
- RootCause dispatch tests assert the child ID is present in audit state.
- Run `cargo fmt -- --check`, `cargo clippy --all-targets -- -D warnings`, and `cargo test --all`.
