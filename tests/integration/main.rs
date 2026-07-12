//! Unified integration test binary.
//!
//! Previously each file in `tests/` compiled as a separate test binary (13
//! binaries), each linking against the full library. Consolidating them into
//! a single binary with modules eliminates 12 redundant link passes, cutting
//! `cargo test` and `cargo clippy --all-targets` compilation time
//! significantly.
//!
//! Test names are namespaced by module, e.g. `integration::tools_test::foo`.
//! `cargo test <name>` still matches by substring.

mod codegraph_mcp_e2e;
mod comet_integration_test;
mod integration_test;
mod package_json_test;
mod plugin_loading_test;
mod refactor_e2e_test;
mod skills_test;
mod strict_subagent_isolation;
mod subagent_evaluation;
mod system_reminder;
mod tools_test;
mod unified_subagent_lifecycle;
mod workflow_comet_test;
