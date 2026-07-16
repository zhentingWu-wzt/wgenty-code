#!/usr/bin/env bash
# Deterministic unit tests for subagent permission pipeline (no API key).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"

echo "== filter_allowed_tools / explore_readonly =="
cargo test --lib explore_readonly_filters_mutating_fs_tools -- --nocapture
cargo test --lib explore_readonly_false_keeps_mutating_tools -- --nocapture

echo
echo "== GuardingToolPort permission pipeline =="
# Module path may be teams::guarding_tool_port::tests::*
cargo test --lib guarding_tool_port -- --nocapture

echo
echo "All targeted permission unit tests finished."
