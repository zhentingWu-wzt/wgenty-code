//! Tasks Module — agent self-tracking (s03) and persistent task management (s07).
//!
//! - `TodoState`/`TodoItem` (s03): session-scoped checklist types used by the
//!   daemon `/todos` endpoint. The `TodoWriteTool` has been removed; task
//!   tracking is now done via the `update_plan` tool.
//! - `TaskManagement` (s07): persistent CRUD tasks with dependency graph.

pub mod management;
pub mod store;
pub mod todo_write;
pub mod types;

pub use management::TaskManagementTool;
pub use todo_write::{SubagentTodoMeta, TodoItem, TodoState};
pub use types::{Task, TaskPriority, TaskStatus};
