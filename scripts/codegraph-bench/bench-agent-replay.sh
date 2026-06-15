#!/usr/bin/env bash
# bench-agent-replay.sh — Replay standard code navigation tasks
# and measure codegraph adoption rate after prompt/description changes.
#
# Usage: bench-agent-replay.sh [--tasks <dir>] [--output <dir>] [--wgenty <bin>]
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

TASKS_DIR="$SCRIPT_DIR/agent-tasks"
OUTPUT_DIR=""
WGENTY_BIN=""
DAEMON_PORT=8371
DAEMON_PID=""
SESSIONS_BEFORE=""
SESSIONS_AFTER=""

usage() {
    cat <<EOF
Usage: bench-agent-replay.sh [OPTIONS]
  --tasks <path>      Directory with nav-*.yaml task files (default: agent-tasks/)
  --output <path>     Output directory (default: results/<timestamp>)
  --wgenty <path>     wgenty-code binary path
  --help              Show this help
EOF
    exit 0
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --tasks) TASKS_DIR="$2"; shift 2 ;;
        --output) OUTPUT_DIR="$2"; shift 2 ;;
        --wgenty) WGENTY_BIN="$2"; shift 2 ;;
        --help) usage ;;
        *) echo "Unknown: $1"; usage ;;
    esac
done

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

# ── Output dir ─────────────────────────────────────────────────────────────
TIMESTAMP=$(date -u +"%Y%m%dT%H%M%SZ")
if [ -z "$OUTPUT_DIR" ]; then
    OUTPUT_DIR="$SCRIPT_DIR/results/$TIMESTAMP"
fi
mkdir -p "$OUTPUT_DIR"

# ── Helpers ────────────────────────────────────────────────────────────────
cleanup() {
    if [ -n "$DAEMON_PID" ] && kill -0 "$DAEMON_PID" 2>/dev/null; then
        kill "$DAEMON_PID" 2>/dev/null || true
        wait "$DAEMON_PID" 2>/dev/null || true
    fi
}
trap cleanup EXIT

# Wait for daemon to be ready
wait_for_daemon() {
    local max_wait=10
    local waited=0
    while [ $waited -lt $max_wait ]; do
        if curl -s "http://localhost:$DAEMON_PORT/api/v1/health" >/dev/null 2>&1; then
            return 0
        fi
        sleep 0.5
        waited=$((waited + 1))
    done
    echo "ERROR: Daemon did not start within ${max_wait}s" >&2
    return 1
}

# List session IDs before/after for diffing
list_session_ids() {
    curl -s "http://localhost:$DAEMON_PORT/api/v1/sessions" 2>/dev/null | \
        jq -r '.[].id // empty' 2>/dev/null | sort
}

# ── Start daemon ───────────────────────────────────────────────────────────
echo "[replay] Starting daemon on port $DAEMON_PORT..."
"$WGENTY_BIN" daemon --port "$DAEMON_PORT" &
DAEMON_PID=$!
wait_for_daemon || exit 1
echo "[replay] Daemon ready (pid=$DAEMON_PID)"

# ── Collect tasks ──────────────────────────────────────────────────────────
shopt -s nullglob
TASK_FILES=("$TASKS_DIR"/nav-*.yaml)
shopt -u nullglob

if [ ${#TASK_FILES[@]} -eq 0 ]; then
    echo "ERROR: No nav-*.yaml files found in $TASKS_DIR" >&2
    exit 1
fi

echo "[replay] Found ${#TASK_FILES[@]} task files"

# ── Per-task replay ────────────────────────────────────────────────────────
declare -a TASK_RESULTS
STRONG_COUNT=0
STRONG_TOTAL=0
OTHER_COUNT=0
OTHER_TOTAL=0

# Strong categories (8 tasks) and other categories (6 tasks)
STRONG=("definition_lookup" "reference_lookup" "call_chain" "impl_enumeration")
OTHER=("module_structure" "cross_module_path")

for task_file in "${TASK_FILES[@]}"; do
    task_id=$(basename "$task_file" .yaml)
    # Extract YAML fields with simple grep (avoid yq dependency)
    category=$(grep '^category:' "$task_file" | sed 's/^category: *//')
    prompt=$(grep '^prompt:' "$task_file" | sed 's/^prompt: *//' | sed 's/^"//;s/"$//')

    echo "[replay] Task: $task_id ($category)"

    # Record sessions before
    SESSIONS_BEFORE=$(list_session_ids)

    # Send chat request via daemon API
    RESPONSE=$(curl -s -X POST "http://localhost:$DAEMON_PORT/api/v1/chat/stream" \
        -H "Content-Type: application/json" \
        -d "{\"messages\":[{\"role\":\"user\",\"content\":\"$prompt\"}]}" \
        --max-time 120 2>&1 || true)

    # Find new session
    SESSIONS_AFTER=$(list_session_ids)
    NEW_SESSION=$(comm -13 <(echo "$SESSIONS_BEFORE") <(echo "$SESSIONS_AFTER") | head -1)

    # Check if codegraph tools were used
    USED_CODEGRAPH=false
    TOOL_SEQUENCE=""
    if [ -n "$NEW_SESSION" ]; then
        # Get session details from daemon API
        SESSION_DATA=$(curl -s "http://localhost:$DAEMON_PORT/api/v1/sessions/$NEW_SESSION" 2>/dev/null || echo "{}")
        TOOL_SEQUENCE=$(echo "$SESSION_DATA" | jq -r '[.messages[]?.tool_calls[]?.function.name // empty] | join(", ")' 2>/dev/null || echo "")

        # Check for codegraph tools in session
        CG_CALLS=$(echo "$SESSION_DATA" | jq -r '[.messages[]?.tool_calls[]? | select(.function.name | test("codegraph"))] | length' 2>/dev/null || echo "0")
        if [ "$CG_CALLS" -gt 0 ]; then
            USED_CODEGRAPH=true
        fi
    else
        TOOL_SEQUENCE="daemon_error"
    fi

    # Record result
    TASK_RESULTS+=("{\"task_id\":\"$task_id\",\"category\":\"$category\",\"used_codegraph\":$USED_CODEGRAPH,\"tool_sequence\":\"$TOOL_SEQUENCE\"}")

    # Aggregate by category type
    if printf '%s\n' "${STRONG[@]}" | grep -qx "$category"; then
        STRONG_TOTAL=$((STRONG_TOTAL + 1))
        [ "$USED_CODEGRAPH" = true ] && STRONG_COUNT=$((STRONG_COUNT + 1))
    else
        OTHER_TOTAL=$((OTHER_TOTAL + 1))
        [ "$USED_CODEGRAPH" = true ] && OTHER_COUNT=$((OTHER_COUNT + 1))
    fi

    sleep 1  # Brief pause between tasks to avoid rate limits
done

# ── Compute rates ──────────────────────────────────────────────────────────
STRONG_RATE="0"
OTHER_RATE="0"
[ "$STRONG_TOTAL" -gt 0 ] && STRONG_RATE=$(python3 -c "print(round($STRONG_COUNT / $STRONG_TOTAL, 4))" 2>/dev/null || echo "0")
[ "$OTHER_TOTAL" -gt 0 ] && OTHER_RATE=$(python3 -c "print(round($OTHER_COUNT / $OTHER_TOTAL, 4))" 2>/dev/null || echo "0")

# ── Write output ───────────────────────────────────────────────────────────
REPORT="$OUTPUT_DIR/agent-replay.json"
(
    echo "{"
    echo "  \"timestamp\": \"$TIMESTAMP\","
    echo "  \"task_count\": ${#TASK_FILES[@]},"
    echo "  \"per_task\": ["
    for i in "${!TASK_RESULTS[@]}"; do
        comma=","
        [ "$i" = "$((${#TASK_RESULTS[@]} - 1))" ] && comma=""
        echo "    ${TASK_RESULTS[$i]}$comma"
    done
    echo "  ],"
    echo "  \"aggregate\": {"
    echo "    \"strong_categories\": {"
    echo "      \"total\": $STRONG_TOTAL,"
    echo "      \"codegraph_count\": $STRONG_COUNT,"
    echo "      \"rate\": $STRONG_RATE"
    echo "    },"
    echo "    \"other_categories\": {"
    echo "      \"total\": $OTHER_TOTAL,"
    echo "      \"codegraph_count\": $OTHER_COUNT,"
    echo "      \"rate\": $OTHER_RATE"
    echo "    }"
    echo "  }"
    echo "}"
) > "$REPORT"

echo ""
echo "=== Results ==="
echo "Strong categories: $STRONG_COUNT/$STRONG_TOTAL ($STRONG_RATE)"
echo "Other categories:  $OTHER_COUNT/$OTHER_TOTAL ($OTHER_RATE)"
echo "Report: $REPORT"

# ── Threshold check ────────────────────────────────────────────────────────
STRONG_PASS=false
OTHER_PASS=false
if python3 -c "exit(0 if $STRONG_RATE >= 0.6 else 1)" 2>/dev/null; then
    STRONG_PASS=true
fi
if python3 -c "exit(0 if $OTHER_RATE >= 0.25 else 1)" 2>/dev/null; then
    OTHER_PASS=true
fi

echo ""
if [ "$STRONG_PASS" = true ] && [ "$OTHER_PASS" = true ]; then
    echo "✅ ALL THRESHOLDS PASSED"
    echo "   Strong ≥60%: $STRONG_PASS"
    echo "   Other  ≥25%: $OTHER_PASS"
    exit 0
else
    echo "⚠️  THRESHOLDS NOT MET"
    echo "   Strong ≥60%: $STRONG_PASS ($STRONG_RATE)"
    echo "   Other  ≥25%: $OTHER_PASS ($OTHER_RATE)"
    exit 1
fi
