//! Subagent dispatch protocol documentation.
//!
//! The comet-workflow-compat protocol for subagent-driven development follows
//! a strict implement → review×2 → fix → commit cycle.
//!
//! ## Protocol Flow
//!
//! 1. **Dispatch** — Coordinator assigns a task to an implementer subagent
//! 2. **Implement** — Implementer writes code following TDD
//! 3. **Review×2** — Two independent reviews:
//!    - Spec compliance review (does the code match the requirements?)
//!    - Code quality review (is the code clean, efficient, maintainable?)
//! 4. **Fix** — Implementer addresses review findings
//! 5. **Commit** — Coordinator verifies all reviews pass and commits
//!
//! ## Task State Machine
//!
//! ```text
//! pending → dispatched → implementing → reviewing → fixing → done
//!                                    ↑           │
//!                                    └───────────┘ (on review failure)
//! ```
//!
//! ## Coordinator Responsibilities
//!
//! - Dispatch tasks to subagent implementers
//! - Do NOT execute tasks directly in the main session
//! - Track progress in `.comet/subagent-progress.md`
//! - Verify both spec compliance and code quality reviews pass
//! - Commit only after dual-review approval
//!
//! ## Implementer Responsibilities
//!
//! - Follow TDD (test-driven development)
//! - Write minimal code to satisfy the task requirements
//! - Report completion with evidence
//! - Address review findings promptly
//!
//! ## Review Criteria
//!
//! ### Spec Compliance
//! - All task requirements are met
//! - No regressions in existing functionality
//! - Tests cover the specified behavior
//!
//! ### Code Quality
//! - Code is clean and readable
//! - No unnecessary complexity
//! - Follows project conventions
//! - No new external dependencies unless approved
