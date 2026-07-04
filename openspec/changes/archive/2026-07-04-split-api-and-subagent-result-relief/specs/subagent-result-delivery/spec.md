## ADDED Requirements

### Requirement: Large subagent results remain accessible without loss
When a subagent produces a result exceeding the persistence threshold, the system SHALL preserve the full content such that the parent agent can access it without lossy truncation. The parent agent SHALL NOT be presented with a fixed-length prefix summary as the only representation of a large result.

#### Scenario: Parent agent can recover full content of a large result
- **WHEN** a subagent produces a result larger than `MAX_INLINE_RESULT_LEN` (4000 chars)
- **THEN** the full content SHALL be persisted to disk
- **AND** the parent agent SHALL be able to access the full content (either inline or via a recovery path)

#### Scenario: Large result not replaced by short prefix-only summary
- **WHEN** a subagent result exceeds the persistence threshold
- **THEN** the parent agent SHALL NOT receive only a 200-character prefix summary as the sole representation
- **AND** the full content SHALL remain recoverable

### Requirement: Large result delivery controls parent context token cost
The system SHALL deliver large subagent results to the parent agent through a mechanism that bounds the parent agent's context token consumption, rather than unconditionally inlining the full content. The specific mechanism (on-demand loading, compaction-time degradation, or hybrid) is determined by design.

#### Scenario: Full content not unconditionally inlined
- **WHEN** a subagent result exceeds the persistence threshold
- **THEN** the system SHALL NOT unconditionally inline the entire content into the parent agent's context as the sole delivery strategy
- **AND** a token-bounding mechanism SHALL be in place

### Requirement: Disk persistence for recovery
When a subagent result exceeds the persistence threshold, the system SHALL persist a copy to the JSONL mailbox so the full content can be recovered later (e.g., after context compaction).

#### Scenario: Large result persisted to disk
- **WHEN** a subagent result exceeds `MAX_INLINE_RESULT_LEN`
- **THEN** a copy SHALL be written to the JSONL mailbox file
- **AND** the recovery path SHALL be communicated to the parent agent

#### Scenario: Persistence failure does not lose content
- **WHEN** disk persistence fails for a large result
- **THEN** the full content SHALL still be returned to the parent agent inline (no truncation)
- **AND** the failure SHALL be logged
