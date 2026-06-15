#!/usr/bin/env bash
# bench-query.sh — Codegraph query latency benchmark
#
# Measures wall-clock time for codegraph_node and codegraph_explore
# against a set of query fixtures, outputting median + p95 per tool.
#
# Usage: bench-query.sh [--target <path>] [--output <dir>] [--wgenty <bin>]
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

source "$SCRIPT_DIR/lib/timing.sh"
source "$SCRIPT_DIR/lib/json-helpers.sh"

# ── Defaults ──────────────────────────────────────────────────────────────
TARGET="."
OUTPUT_DIR=""
WGENTY_BIN=""
PORT=0

usage() {
    cat <<EOF
Usage: bench-query.sh [OPTIONS]
  --target <path>   Target Rust project with codegraph index (default: .)
  --output <path>   Output directory (default: results/<timestamp>)
  --wgenty <path>   wgenty-code binary path
  --help            Show this help
EOF
    exit 0
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --target) TARGET="$2"; shift 2 ;;
        --output) OUTPUT_DIR="$2"; shift 2 ;;
        --wgenty) WGENTY_BIN="$2"; shift 2 ;;
        --help) usage ;;
        *) echo "Unknown: $1"; usage ;;
    esac
done

# ── Resolve target to absolute path ───────────────────────────────────────
TARGET="$(cd "$TARGET" && pwd)"

# ── Binary resolution ─────────────────────────────────────────────────────
if [ -z "$WGENTY_BIN" ]; then
    if [ -f "$PROJECT_ROOT/target/release/wgenty-code" ]; then
        WGENTY_BIN="$PROJECT_ROOT/target/release/wgenty-code"
    elif command -v wgenty-code &>/dev/null; then
        WGENTY_BIN="wgenty-code"
    else
        echo "ERROR: wgenty-code not found. Build with: cargo build --release" >&2
        exit 1
    fi
fi
if [ ! -x "$WGENTY_BIN" ] && ! command -v "$WGENTY_BIN" &>/dev/null; then
    echo "ERROR: Binary not executable: $WGENTY_BIN" >&2
    exit 1
fi

# ── Pre-checks ────────────────────────────────────────────────────────────
INDEX_DB="$TARGET/.codegraph/index.db"
if [ ! -f "$INDEX_DB" ]; then
    echo "ERROR: No codegraph index at $INDEX_DB" >&2
    echo "  Run: wgenty-code codegraph index" >&2
    exit 1
fi

QUERIES_FILE="$SCRIPT_DIR/query-fixtures/codegraph-queries.txt"
if [ ! -f "$QUERIES_FILE" ]; then
    echo "ERROR: Queries file not found: $QUERIES_FILE" >&2
    exit 1
fi

# ── Output directory ──────────────────────────────────────────────────────
TIMESTAMP=$(date -u +"%Y%m%dT%H%M%SZ")
[ -z "$OUTPUT_DIR" ] && OUTPUT_DIR="$SCRIPT_DIR/results/$TIMESTAMP"
mkdir -p "$OUTPUT_DIR"

echo "=== bench-query.sh ==="
echo "Target: $TARGET"
echo "Output: $OUTPUT_DIR"
echo "Binary: $WGENTY_BIN"

# ── Read query fixtures ───────────────────────────────────────────────────
QUERIES=()
while IFS= read -r line; do
    line="${line#"${line%%[![:space:]]*}"}"
    line="${line%"${line##*[![:space:]]}"}"
    [ -n "$line" ] && QUERIES+=("$line")
done < "$QUERIES_FILE"
echo "Queries: ${#QUERIES[@]}"

# ── Determine port ────────────────────────────────────────────────────────
if [ "$PORT" = "0" ]; then
    PORT=$(( ((RANDOM << 15) | RANDOM) % 40000 + 20000 ))
fi
DAEMON_URL="http://127.0.0.1:$PORT"

# ── Start daemon ──────────────────────────────────────────────────────────
(cd "$TARGET" && "$WGENTY_BIN" daemon --port "$PORT") &
DAEMON_PID=$!

echo -n "Starting daemon on port $PORT..."
DAEMON_READY=false
for _ in $(seq 1 60); do
    if curl -sf "$DAEMON_URL/api/v1/health" >/dev/null 2>&1; then
        DAEMON_READY=true
        echo " ready"
        break
    fi
    sleep 0.2
done

if [ "$DAEMON_READY" = false ]; then
    echo " FAILED"
    echo "ERROR: Daemon did not start within 12s" >&2
    kill "$DAEMON_PID" 2>/dev/null || true
    exit 1
fi

# ── Cleanup trap ──────────────────────────────────────────────────────────
cleanup() {
    kill "$DAEMON_PID" 2>/dev/null || true
    wait "$DAEMON_PID" 2>/dev/null || true
}
trap cleanup EXIT

# ── Tool invocation helper ────────────────────────────────────────────────
# Calls the daemon HTTP API to execute a tool, returning response JSON.
invoke_tool() {
    local tool_name="$1"
    local field_name="$2"
    local value="$3"
    local body
    body=$(jq -n \
        --arg name "$tool_name" \
        --arg field "$field_name" \
        --arg val "$value" \
        '{"tool_name": $name, "arguments": {($field): $val}}')
    curl -sf --max-time 30 -X POST "$DAEMON_URL/api/v1/tools/execute" \
        -H "Content-Type: application/json" \
        -d "$body" 2>/dev/null
}

# ── Warmup ────────────────────────────────────────────────────────────────
# First call triggers lazy engine init (ENGINE OnceLock) — warm it up.
echo -n "Warming up..."
invoke_tool "codegraph_node" "symbol" "Tool" >/dev/null 2>&1 || true
invoke_tool "codegraph_explore" "query" "Tool" >/dev/null 2>&1 || true
echo " done"

# ── Benchmark loop ────────────────────────────────────────────────────────
echo "Benchmarking..."
NODE_SAMPLES=()
EXPLORE_SAMPLES=()

for query in "${QUERIES[@]}"; do
    # codegraph_node
    echo -n "  node($query)... "
    start=$(date +%s%N)
    invoke_tool "codegraph_node" "symbol" "$query" >/dev/null 2>&1 || true
    end=$(date +%s%N)
    elapsed_ms=$(( (end - start) / 1000000 ))
    elapsed_s=$(echo "scale=3; $elapsed_ms / 1000" | bc)
    NODE_SAMPLES+=("$elapsed_s")
    echo "${elapsed_ms}ms"

    # codegraph_explore
    echo -n "  explore($query)... "
    start=$(date +%s%N)
    invoke_tool "codegraph_explore" "query" "$query" >/dev/null 2>&1 || true
    end=$(date +%s%N)
    elapsed_ms=$(( (end - start) / 1000000 ))
    elapsed_s=$(echo "scale=3; $elapsed_ms / 1000" | bc)
    EXPLORE_SAMPLES+=("$elapsed_s")
    echo "${elapsed_ms}ms"
done

# ── Compute percentiles & write output ────────────────────────────────────
echo -n "Computing percentiles..."
node_csv=$(IFS=,; echo "${NODE_SAMPLES[*]}")
explore_csv=$(IFS=,; echo "${EXPLORE_SAMPLES[*]}")

node_json=$(numbers_to_json "$node_csv")
explore_json=$(numbers_to_json "$explore_csv")

node_result=$(compute_percentiles "$node_json")
explore_result=$(compute_percentiles "$explore_json")

jq -n \
    --argjson node "$node_result" \
    --argjson explore "$explore_result" \
    '{
        "codegraph_node": {samples: $node.samples, median: $node.median, p95: $node.p95},
        "codegraph_explore": {samples: $explore.samples, median: $explore.median, p95: $explore.p95}
    }' > "$OUTPUT_DIR/query-perf.json"
echo " done"

echo "=== Done ==="
echo "Results: $OUTPUT_DIR/query-perf.json"
