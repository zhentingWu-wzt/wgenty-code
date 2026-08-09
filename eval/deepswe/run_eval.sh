#!/usr/bin/env bash
# DeepSWE evaluation runner for wgenty-code.
#
# Usage:
#   ./run_eval.sh <deep-swe-dir> [n_tasks] [extra pier args...]
#
# Arguments:
#   deep-swe-dir  Path to the cloned deep-swe repository (contains tasks/ dir)
#   n_tasks       Number of tasks to run (default: 10, use 0 for all 113)
#
# Prerequisites:
#   1. wgenty-code Linux binary at repo root (wgenty-code-linux-amd64)
#      Build: cargo zigbuild --release --target x86_64-unknown-linux-musl --features bundled-sqlite
#   2. Pier installed: uv tool install datacurve-pier
#   3. DEEPSEEK_API_KEY (or ANTHROPIC_API_KEY) env var set
#   4. DeepSWE repo cloned: git clone https://github.com/datacurve-ai/deep-swe
#
# Example:
#   DEEPSEEK_API_KEY=sk-xxx ./run_eval.sh ~/project/deep-swe 1
#   DEEPSEEK_API_KEY=sk-xxx ./run_eval.sh ~/project/deep-swe 10

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

DEEPSWE_DIR="${1:?Usage: $0 <deep-swe-dir> [n_tasks]}"
N_TASKS="${2:-10}"
shift 2 || true

# --- Determine API key + provider env var ---
API_ENV=""
if [ -n "${DEEPSEEK_API_KEY:-}" ]; then
    API_ENV="--ae DEEPSEEK_API_KEY=${DEEPSEEK_API_KEY}"
elif [ -n "${ANTHROPIC_API_KEY:-}" ]; then
    API_ENV="--ae ANTHROPIC_API_KEY=${ANTHROPIC_API_KEY}"
elif [ -n "${DASHSCOPE_API_KEY:-}" ]; then
    API_ENV="--ae DASHSCOPE_API_KEY=${DASHSCOPE_API_KEY}"
else
    echo "ERROR: Set DEEPSEEK_API_KEY, ANTHROPIC_API_KEY, or DASHSCOPE_API_KEY" >&2
    exit 1
fi

# --- Check binary exists ---
BINARY="$REPO_ROOT/wgenty-code-linux-amd64"
if [ ! -f "$BINARY" ]; then
    echo "ERROR: Linux binary not found at $BINARY" >&2
    echo "Build it with:" >&2
    echo "  export PATH=\"/tmp/zig-macos-aarch64-0.14.0:\$PATH\"" >&2
    echo "  cargo zigbuild --release --target x86_64-unknown-linux-musl --features bundled-sqlite" >&2
    echo "  cp target/x86_64-unknown-linux-musl/release/wgenty-code $BINARY" >&2
    exit 1
fi

# --- Check deep-swe tasks dir ---
TASKS_DIR="$DEEPSWE_DIR/tasks"
if [ ! -d "$TASKS_DIR" ]; then
    echo "ERROR: tasks/ directory not found in $DEEPSWE_DIR" >&2
    exit 1
fi

# --- Generate job.yaml with actual paths ---
TMP_JOB="/tmp/wgenty-deepswe-job-$$.yaml"
sed \
    -e "s|__WGENTY_BINARY__|$BINARY|g" \
    -e "s|__DEEPSWE_TASKS__|$TASKS_DIR|g" \
    "$SCRIPT_DIR/job.yaml" > "$TMP_JOB"

# --- Run ---
export PYTHONPATH="$SCRIPT_DIR:${PYTHONPATH:-}"

if [ "$N_TASKS" = "0" ]; then
    # Run all tasks (no -l limit)
    pier run -c "$TMP_JOB" $API_ENV -p "$TASKS_DIR" "$@"
else
    pier run -c "$TMP_JOB" $API_ENV -p "$TASKS_DIR" -l "$N_TASKS" "$@"
fi

# Cleanup
rm -f "$TMP_JOB"
