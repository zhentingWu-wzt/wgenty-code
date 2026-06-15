# rlm-budget-control Specification

## Purpose
Enable per-subagent token budget allocation with hard cutoffs to prevent runaway cost in recursive multi-agent execution.

## ADDED Requirements

### Requirement: Token budget parameter on task tool
The `task` tool input schema SHALL include an optional `token_budget` parameter (in thousands of tokens) that limits the subagent's total token consumption.

#### Scenario: Budget specified and enforced
- **WHEN** a subagent is spawned with `token_budget = 10` (10k tokens)
- **AND** the subagent's cumulative token usage reaches 10,000 tokens
- **THEN** the subagent loop SHALL immediately stop and return an error: "Token budget exceeded (limit: 10k, used: 10k)"

#### Scenario: No budget specified
- **WHEN** a subagent is spawned without a `token_budget` parameter (or `token_budget = 0`)
- **THEN** token consumption SHALL NOT be limited by budget (other limits like max_rounds and timeout still apply)

#### Scenario: Budget check on each API round
- **WHEN** a subagent completes an API round
- **THEN** the cumulative token count SHALL be checked against the budget before executing any tool calls from that round

### Requirement: Default budget configuration
The system SHALL support a global default token budget via `default_subagent_token_budget_k` in Settings.

#### Scenario: Default budget used when not explicitly set
- **WHEN** a subagent is spawned without an explicit `token_budget`
- **AND** `settings.default_subagent_token_budget_k` is set to 50
- **THEN** the subagent SHALL be subject to a 50k token budget

#### Scenario: Explicit budget overrides default
- **WHEN** a subagent is spawned with `token_budget = 20`
- **AND** `settings.default_subagent_token_budget_k` is set to 50
- **THEN** the 20k budget SHALL be enforced, overriding the default

### Requirement: RLM pipeline budget distribution
When the RLM pipeline is used, the total budget SHALL be distributed across planner, executor, and aggregator phases.

#### Scenario: Budget distributed across pipeline phases
- **WHEN** a delegate task is called with `token_budget = 100`
- **THEN** the planner SHALL receive up to 10% (10k), sub-tasks SHALL evenly split 80% (80k / N), and the aggregator SHALL receive up to 10% (10k)

#### Scenario: Unused budget rolls forward
- **WHEN** the planner phase uses only 5k of its 10k allocation
- **THEN** the unused 5k SHALL be added to the sub-task pool

### Requirement: Budget exhaustion is reported
When a subagent is killed due to budget exhaustion, the error SHALL include actionable diagnostics.

#### Scenario: Budget exhaustion error details
- **WHEN** a subagent exceeds its token budget
- **THEN** the error message SHALL include: limit, actual usage, number of rounds completed, and the last tool being executed
