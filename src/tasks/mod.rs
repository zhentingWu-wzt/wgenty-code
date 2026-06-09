//! Tasks Module — agent self-tracking (s03) and persistent task management (s07).
//!
//! - `TodoWrite` (s03): session-scoped checklist the agent updates as a batch.
//!   Max 20 items, only 1 in_progress, nag reminder in the agent loop.
//! - `TaskManagement` (s07): persistent CRUD tasks with dependency graph.

pub mod management;
pub mod todo_write;
pub mod types;

pub use management::TaskManagementTool;
pub use todo_write::{TodoItem, TodoState, TodoWriteTool};
pub use types::{Task, TaskPriority, TaskStatus};
