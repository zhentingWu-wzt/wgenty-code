//! Tasks Module — file-based task management with dependency tracking.
//!
//! Corresponds to harness mechanisms s07+s08: break goals into ordered tasks,
//! persist to disk, background daemon execution with notification injection.

pub mod management;

pub use management::TaskManagementTool;
