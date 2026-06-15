# rlm-structured-reduction Specification

## Purpose
Replace natural-language subagent result aggregation with structured data formats (claims for analysis, unified diff for code changes) to enable deterministic conflict detection and merging.

## ADDED Requirements

### Requirement: Subagent output format selection
The RLM pipeline SHALL select between `structured-claims` and `unified-diff` output formats based on the sub-task type.

#### Scenario: Analysis/exploration task uses claims format
- **WHEN** a sub-task is classified as analysis or exploration (e.g., searching codebase, investigating a bug)
- **THEN** the subagent SHALL be instructed to output results in `structured-claims/1` format

#### Scenario: Code modification task uses diff format
- **WHEN** a sub-task is classified as code modification (e.g., refactoring, implementing a feature)
- **THEN** the subagent SHALL be instructed to output results in `unified-diff/1` format

#### Scenario: Mixed task uses both formats
- **WHEN** a sub-task involves both analysis and code changes
- **THEN** the subagent SHALL output claims for the analysis portion and diffs for the code changes, each in their respective sections

### Requirement: Structured claims format
Analysis subagent results SHALL conform to the `structured-claims/1` JSON schema.

#### Scenario: Valid claims output
- **WHEN** a subagent produces analysis results
- **THEN** the output SHALL be a JSON object with `format: "structured-claims/1"` and a `claims` array, where each claim has `id`, `claim`, `evidence`, `confidence` (0.0-1.0), `conflicts_with` (array of claim IDs), `actionable` (boolean), and optional `recommendation`

#### Scenario: Claims with conflict detection
- **WHEN** a subagent identifies a finding that contradicts another claim
- **THEN** the claim's `conflicts_with` array SHALL reference the conflicting claim's `id`

#### Scenario: Claims confidence is numeric
- **WHEN** a subagent is uncertain about a finding
- **THEN** the `confidence` field SHALL reflect the uncertainty as a float between 0.0 and 1.0, not as a text label

### Requirement: Unified diff format
Code modification subagent results SHALL conform to the `unified-diff/1` JSON schema.

#### Scenario: Valid diff output
- **WHEN** a subagent produces code changes
- **THEN** the output SHALL be a JSON object with `format: "unified-diff/1"` and a `changes` array, where each change has `file`, `intent`, `diff` (unified diff string), `confidence` (0.0-1.0), and `depends_on` (array of file paths)

#### Scenario: Multiple files changed
- **WHEN** a subagent modifies multiple files
- **THEN** each file's change SHALL be a separate entry in the `changes` array with its own `intent` and `diff`

### Requirement: Aggregator merges structured results
The RLM Aggregator SHALL merge structured sub-task results deterministically before falling back to LLM synthesis.

#### Scenario: Claims deduplication by text similarity
- **WHEN** two sub-tasks produce claims with Jaccard similarity > 0.8 on the `claim` text
- **THEN** the Aggregator SHALL merge them into one claim, keeping the higher confidence value and combining evidence

#### Scenario: Conflict detection from conflicts_with
- **WHEN** any claim's `conflicts_with` array references another claim by ID
- **THEN** the Aggregator SHALL mark both claims as `status: conflicted` and present them for resolution

#### Scenario: Diff conflict detection by file path
- **WHEN** two sub-tasks produce changes for the same file path
- **THEN** the Aggregator SHALL mark those changes as `status: potential_write_conflict` and include both in the final output for review

#### Scenario: Fallback to LLM aggregation
- **WHEN** sub-task results cannot be parsed as valid structured output
- **THEN** the Aggregator SHALL fall back to the existing LLM-based merge with a warning in the output metadata
