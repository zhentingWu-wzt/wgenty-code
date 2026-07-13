//! Startup timing instrumentation.
//!
//! Records a high-resolution baseline at the earliest point in `main()` and
//! exposes [`mark()`] to log elapsed milliseconds at each startup milestone.
//! All output goes through `tracing` (i.e. `~/.wgenty-code/logs/wgenty-code.log`).
//!
//! Log lines look like:
//! `startup timing | <name> | +<elapsed_ms>ms`

use std::sync::OnceLock;
use std::time::Instant;

static BASELINE: OnceLock<Instant> = OnceLock::new();

/// Record the startup baseline.
///
/// Must be called once at the very top of `main()`, before any other work — and
/// before `logging::init()` — so the baseline reflects the true process entry
/// point. Safe to call multiple times; only the first call wins.
pub fn init() {
    let _ = BASELINE.get_or_init(Instant::now);
}

/// Elapsed milliseconds since [`init()`] was called. Returns `0` if
/// [`init()`] has not been called yet.
pub fn elapsed_ms() -> u128 {
    BASELINE.get().map(|t| t.elapsed().as_millis()).unwrap_or(0)
}

/// Log a startup milestone at `info` level:
/// `startup timing | <name> | +<elapsed_ms>ms`.
///
/// Call this after each meaningful startup phase to build a timeline in the log.
pub fn mark(name: &str) {
    tracing::info!("startup timing | {name} | +{}ms", elapsed_ms());
}
