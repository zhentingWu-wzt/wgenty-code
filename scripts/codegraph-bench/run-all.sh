#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
TARGET="."
OUTPUT_DIR=""
REPEATS=1
SKIP_AGENT=false
WGENTY_BIN="${WGENTY_BIN:-}"

usage() {
  cat <<EOF
Usage: run-all.sh [OPTIONS]
  --target <path>      Target Rust project (default: .)
  --output <path>      Output directory (default: results/<timestamp>)
  --repeats <n>        Repeat count for stability (default: 1)
  --skip-agent         Skip agent usage measurement
  --help               Show this help
EOF
  exit 0
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --target) TARGET="$2"; shift 2 ;;
    --output) OUTPUT_DIR="$2"; shift 2 ;;
    --repeats) REPEATS="$2"; shift 2 ;;
    --skip-agent) SKIP_AGENT=true; shift ;;
    --help) usage ;;
    *) echo "Unknown: $1"; usage ;;
  esac
done

# Binary check
if [ -z "$WGENTY_BIN" ]; then
  if [ -f "$SCRIPT_DIR/../../target/release/wgenty-code" ]; then
    WGENTY_BIN="$SCRIPT_DIR/../../target/release/wgenty-code"
  elif command -v wgenty-code &>/dev/null; then
    WGENTY_BIN="wgenty-code"
  else
    echo "ERROR: wgenty-code not found. Build with: cargo build --release" >&2
    exit 1
  fi
fi

TIMESTAMP=$(date -u +"%Y%m%dT%H%M%SZ")
if [ -z "$OUTPUT_DIR" ]; then
  OUTPUT_DIR="$SCRIPT_DIR/results/$TIMESTAMP"
fi
mkdir -p "$OUTPUT_DIR"

source "$SCRIPT_DIR/lib/env-fingerprint.sh"
source "$SCRIPT_DIR/lib/json-helpers.sh"
source "$SCRIPT_DIR/lib/timing.sh"

echo "=== Codegraph Baseline Benchmark ==="
echo "Target: $TARGET"
echo "Output: $OUTPUT_DIR"
echo "Binary: $WGENTY_BIN"

fingerprint_env "$OUTPUT_DIR" "$TARGET"

# Sub-scripts (run even if some fail, but track exit codes)
FAILURES=0

if [ -f "$SCRIPT_DIR/bench-perf.sh" ]; then
  /bin/bash "$SCRIPT_DIR/bench-perf.sh" --target "$TARGET" --output "$OUTPUT_DIR" --wgenty "$WGENTY_BIN" --repeats "$REPEATS" || { echo "[WARN] bench-perf.sh failed"; FAILURES=$((FAILURES+1)); }
else
  echo "[SKIP] bench-perf.sh not found (not yet implemented)"
fi

if [ -f "$SCRIPT_DIR/bench-coverage.sh" ]; then
  /bin/bash "$SCRIPT_DIR/bench-coverage.sh" --target "$TARGET" --output "$OUTPUT_DIR" --wgenty "$WGENTY_BIN" || { echo "[WARN] bench-coverage.sh failed"; FAILURES=$((FAILURES+1)); }
else
  echo "[SKIP] bench-coverage.sh not found (not yet implemented)"
fi

if [ "$SKIP_AGENT" = false ] && [ -f "$SCRIPT_DIR/bench-agent.sh" ]; then
  /bin/bash "$SCRIPT_DIR/bench-agent.sh" --wgenty "$WGENTY_BIN" --output "$OUTPUT_DIR" || { echo "[WARN] bench-agent.sh failed"; FAILURES=$((FAILURES+1)); }
fi

if [ -f "$SCRIPT_DIR/gen-report.sh" ]; then
  /bin/bash "$SCRIPT_DIR/gen-report.sh" --output "$OUTPUT_DIR" || { echo "[WARN] gen-report.sh failed"; FAILURES=$((FAILURES+1)); }
else
  echo "[SKIP] gen-report.sh not found (not yet implemented)"
fi

echo "=== Done: $FAILURES failures ==="
exit $FAILURES
