## ADDED Requirements

### Requirement: Replan phase budget allocation
When replan is enabled and a replan is triggered, the replanner API call and the re-execution of replacement sub-tasks SHALL consume budget from the executor phase pool of the `BudgetAllocation`. The total replan consumption (replanner call + replacement sub-task executions) SHALL NOT cause the pipeline to exceed the overall `token_budget_k` limit of the invoking `delegate` call. Unused replan budget SHALL roll forward to the aggregator phase using the same `rollover_unused` semantics as the executor phase.

#### Scenario: Replan consumes executor pool budget
- **WHEN** a replan is triggered and the replanner API call plus replacement sub-task executions consume X tokens
- **THEN** X tokens SHALL be deducted from the executor phase pool of the `BudgetAllocation`
- **AND** the overall pipeline token consumption SHALL NOT exceed the `delegate` call's `token_budget_k` limit

#### Scenario: Unused replan budget rolls to aggregator
- **WHEN** a replan consumes less than its allocated portion of the executor pool
- **THEN** the unused remainder SHALL roll forward to the aggregator phase via `rollover_unused`

#### Scenario: Budget exhaustion prevents replan
- **WHEN** a sub-task fails and the remaining executor pool budget is insufficient for a replanner API call
- **THEN** the Executor SHALL NOT attempt a replan
- **AND** the failed sub-task SHALL be recorded as `[ERROR]`
