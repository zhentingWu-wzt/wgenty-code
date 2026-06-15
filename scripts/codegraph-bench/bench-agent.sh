#!/usr/bin/env bash
# bench-agent.sh -- Agent codegraph usage baseline measurement
#
# Parses repl session JSON to measure codegraph tool adoption rate.
# Supports single session analysis and batch scanning of sessions dir.
#
# Usage:
#   bench-agent.sh --session <path>              # single session
#   bench-agent.sh --sessions-dir <dir>           # batch scan
#   bench-agent.sh --sessions-dir <dir> --tasks <dir>  # cross-ref with tasks
#   bench-agent.sh [--output <dir>]
#
# Output: <output-dir>/agent.json
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

# shellcheck source=lib/json-helpers.sh
source "$SCRIPT_DIR/lib/json-helpers.sh"

# -- Defaults -----------------------------------------------------------------
SESSION=""
SESSIONS_DIR="${SESSIONS_DIR:-$HOME/.wgenty-code/sessions}"
OUTPUT_DIR=""
TASKS_DIR=""

usage() {
    cat <<EOF
Usage: bench-agent.sh [OPTIONS]

  --session <path>       Single session JSON file to analyze
  --sessions-dir <path>  Session directory (default: ~/.wgenty-code/sessions)
  --output <path>        Output directory (default: results/<timestamp>)
  --tasks <path>         Agent tasks directory (for cross-referencing)
  --wgenty <path>        Ignored (compatibility with run-all.sh orchestration)
  --help                 Show this help

Examples:
  bench-agent.sh --session ~/.wgenty-code/sessions/abc.json
  bench-agent.sh --sessions-dir ~/.wgenty-code/sessions
  bench-agent.sh --sessions-dir ~/.wgenty-code/sessions --tasks agent-tasks
EOF
    exit 0
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --session) SESSION="$2"; shift 2 ;;
        --sessions-dir) SESSIONS_DIR="$2"; shift 2 ;;
        --output) OUTPUT_DIR="$2"; shift 2 ;;
        --tasks) TASKS_DIR="$2"; shift 2 ;;
        --wgenty) WGENTY_BIN="$2"; shift 2 ;;  # accepted but not used (session analysis doesn't need the binary)
        --help) usage ;;
        *) echo "Unknown: $1"; usage ;;
    esac
done

# -- Output directory ---------------------------------------------------------
TIMESTAMP=$(date -u +"%Y%m%dT%H%M%SZ")
[ -z "$OUTPUT_DIR" ] && OUTPUT_DIR="$SCRIPT_DIR/results/$TIMESTAMP"
mkdir -p "$OUTPUT_DIR"

echo "=== bench-agent.sh ==="
echo "Output: $OUTPUT_DIR"

# -- Task cache ---------------------------------------------------------------
# Load all task YAML prompts into a JSON map: { "nav-001": "prompt text", ... }
TASKS_JSON="{}"
if [ -n "$TASKS_DIR" ]; then
    TASKS_DIR="$(cd "$TASKS_DIR" && pwd)"
    echo "Tasks dir: $TASKS_DIR"
    if [ -d "$TASKS_DIR" ]; then
        # Build a map from task_id -> prompt by parsing YAML.
        # Uses jq to accumulate entries since bash lacks associative arrays
        # on older versions (macOS bundled bash 3.x).
        current_id=""
        current_prompt=""
        in_prompt=false
        while IFS= read -r line; do
            if [[ "$line" =~ ^task_id:[[:space:]]*(.+) ]]; then
                # Save previous entry
                if [ -n "$current_id" ] && [ -n "$current_prompt" ]; then
                    prompt_escaped=$(echo "$current_prompt" | jq -Rs .)
                    TASKS_JSON=$(echo "$TASKS_JSON" | jq --arg id "$current_id" --argjson prompt "$prompt_escaped" '. + {($id): $prompt}')
                fi
                current_id="${BASH_REMATCH[1]}"
                current_prompt=""
                in_prompt=false
            elif [[ "$line" =~ ^prompt:[[:space:]]*(.*) ]]; then
                current_prompt="${BASH_REMATCH[1]}"
                in_prompt=true
            elif $in_prompt && [[ "$line" =~ ^[[:space:]]+(.*) ]]; then
                # Continuation of multi-line prompt (folded)
                current_prompt="$current_prompt ${BASH_REMATCH[1]}"
            else
                in_prompt=false
            fi
        done < <(cat "$TASKS_DIR"/*.yaml 2>/dev/null)
        # Save last entry
        if [ -n "$current_id" ] && [ -n "$current_prompt" ]; then
            prompt_escaped=$(echo "$current_prompt" | jq -Rs .)
            TASKS_JSON=$(echo "$TASKS_JSON" | jq --arg id "$current_id" --argjson prompt "$prompt_escaped" '. + {($id): $prompt}')
        fi
        echo "  Loaded $(echo "$TASKS_JSON" | jq 'length') tasks"
    else
        echo "  WARNING: tasks dir not found: $TASKS_DIR" >&2
    fi
fi

# -- Analysis helpers ---------------------------------------------------------

# Analyze a single session JSON file and produce per-session JSON record.
# Usage: analyze_session <filepath>
# Output: JSON object with session_id, name, tool_calls[], used_codegraph, matched_task
analyze_session() {
    local file="$1"

    # Check file exists
    [ -f "$file" ] || { echo "null"; return; }

    local sid name_seq tc_seq
    sid=$(jq -r '.id // "unknown"' "$file" 2>/dev/null || echo "unknown")

    # Session name
    name_seq=$(jq -r '.name // ""' "$file" 2>/dev/null || echo "")

    # Has messages?
    local has_msgs
    has_msgs=$(jq 'has("messages")' "$file" 2>/dev/null || echo "false")
    if [ "$has_msgs" != "true" ]; then
        echo "null"
        return
    fi

    # Extract sequential tool call function names
    tc_seq=$(jq -c '[.messages[]?.tool_calls[]? | .function.name]' "$file" 2>/dev/null || echo "[]")

    local tc_count used_cg
    tc_count=$(echo "$tc_seq" | jq 'length')
    used_cg=$(echo "$tc_seq" | jq '[.[] | test("codegraph")] | any')

    # Match against tasks
    # Strategy: for each task, check if the session name or first user message
    # contains the task prompt text (or vice versa), using substring matching.
    # This handles both directions since session names can be abbreviations or
    # full sentences.
    local matched_task=null
    if [ "$TASKS_JSON" != "{}" ] && [ -n "$name_seq" ]; then
        # Normalize for case-insensitive matching
        local name_lc
        name_lc=$(echo "$name_seq" | tr '[:upper:]' '[:lower:]')

        matched_task=$(echo "$TASKS_JSON" | jq -r \
            --arg name_lc "$name_lc" \
            --arg name_orig "$name_seq" \
            '[to_entries[] |
              select(
                (.value | ascii_downcase | contains($name_lc)) or
                ($name_orig | contains(.value))
              ) | .key] |
             if length > 0 then first else null end' \
            2>/dev/null || echo "null")

        # If no match by name, try matching by first user message
        if [ "$matched_task" = "null" ]; then
            local first_user
            first_user=$(jq -r '[.messages[] | select(.role == "user") | .content // ""] | first' "$file" 2>/dev/null || echo "")
            if [ -n "$first_user" ]; then
                local first_user_lc
                first_user_lc=$(echo "$first_user" | tr '[:upper:]' '[:lower:]')
                matched_task=$(echo "$TASKS_JSON" | jq -r \
                    --arg msg_lc "$first_user_lc" \
                    --arg msg_orig "$first_user" \
                    '[to_entries[] |
                      select(
                        (.value | ascii_downcase | contains($msg_lc)) or
                        ($msg_orig | contains(.value))
                      ) | .key] |
                     if length > 0 then first else null end' \
                    2>/dev/null || echo "null")
            fi
        fi
    fi

    # Build per-session JSON record
    jq -c -n \
        --arg id "$sid" \
        --arg name "$name_seq" \
        --argjson tc "$tc_seq" \
        --argjson used "$used_cg" \
        --argjson task "${matched_task:-null}" \
        '{
            session_id: $id,
            name: $name,
            tool_calls: $tc,
            used_codegraph: $used,
            matched_task: $task
        }'
}

# -- Single session mode ------------------------------------------------------
if [ -n "$SESSION" ]; then
    echo "Mode: single session"
    echo "Session: $SESSION"

    if [ ! -f "$SESSION" ]; then
        echo "ERROR: session file not found: $SESSION" >&2
        exit 1
    fi

    record=$(analyze_session "$SESSION")
    if [ "$record" = "null" ]; then
        echo "ERROR: could not parse session (missing messages?)" >&2
        exit 1
    fi

    tool_calls=$(echo "$record" | jq '[.tool_calls[]]')
    by_tool=$(echo "$tool_calls" | jq 'group_by(.) | map({key: .[0], value: length}) | from_entries')
    total_tc=$(echo "$tool_calls" | jq 'length')
    cg_count=$(echo "$tool_calls" | jq '[.[] | select(test("codegraph"))] | length')
    used_cg=$(echo "$record" | jq '.used_codegraph')

    summary=$(jq -n \
        --argjson total 1 \
        --argjson with_cg "$( [ "$used_cg" = "true" ] && echo 1 || echo 0 )" \
        --argjson total_tc "$total_tc" \
        --argjson cg_count "$cg_count" \
        '{
            total_sessions: $total,
            sessions_with_codegraph: $with_cg,
            adoption_rate_pct: (if $total > 0 then ($with_cg * 100 / $total) else 0 end),
            total_tool_calls: $total_tc,
            codegraph_tool_calls: $cg_count,
            codegraph_share_pct: (if $total_tc > 0 then ($cg_count * 10000 / $total_tc | . / 100) else 0 end)
        }')

    jq -n \
        --argjson summary "$summary" \
        --argjson by_tool "$by_tool" \
        --argjson per_session "[$record]" \
        '{summary: $summary, by_tool: $by_tool, per_session: $per_session}' \
        > "$OUTPUT_DIR/agent.json"

    echo "Done -- single session analyzed"
    echo "  Tool calls: $total_tc, codegraph: $cg_count"
    exit 0
fi

# -- Batch mode ---------------------------------------------------------------
echo "Mode: batch scan"
echo "Sessions dir: $SESSIONS_DIR"

if [ ! -d "$SESSIONS_DIR" ]; then
    echo "ERROR: sessions directory not found: $SESSIONS_DIR" >&2
    exit 1
fi

# Collect session files
SESSION_FILES=()
while IFS= read -r -d '' f; do
    SESSION_FILES+=("$f")
done < <(find "$SESSIONS_DIR" -maxdepth 1 -name '*.json' -print0 2>/dev/null)

total="${#SESSION_FILES[@]}"
echo "Sessions found: $total"

if [ "$total" -eq 0 ]; then
    echo "ERROR: no session files found in $SESSIONS_DIR" >&2
    exit 1
fi

# Analyze each session
per_session_json="[]"
sessions_with_cg=0
total_tc=0
total_cg=0
tool_accum="{}"
processed=0
skipped=0

for f in "${SESSION_FILES[@]}"; do
    record=$(analyze_session "$f")
    if [ "$record" = "null" ]; then
        skipped=$((skipped + 1))
        continue
    fi

    processed=$((processed + 1))

    # Accumulate tool calls
    tc_list=$(echo "$record" | jq '[.tool_calls[]]')
    tc_count=$(echo "$tc_list" | jq 'length')
    total_tc=$((total_tc + tc_count))

    # Count codegraph
    cg_in_session=$(echo "$tc_list" | jq '[.[] | select(test("codegraph"))] | length')
    total_cg=$((total_cg + cg_in_session))

    if [ "$cg_in_session" -gt 0 ]; then
        sessions_with_cg=$((sessions_with_cg + 1))
    fi

    # Accumulate per-tool counts
    tool_accum=$(echo "$tool_accum" | jq --argjson tc "$tc_list" '
        reduce $tc[] as $tool (.;
            if has($tool) then .[$tool] += 1 else .[$tool] = 1 end
        )
    ')

    per_session_json=$(echo "$per_session_json" | jq --argjson rec "$record" '. + [$rec]')
done

# -- Compute aggregate summary ------------------------------------------------
adoption_pct=0
if [ "$processed" -gt 0 ]; then
    adoption_pct=$(echo "scale=2; $sessions_with_cg * 100 / $processed" | bc 2>/dev/null || echo "0")
fi

cg_share=0
if [ "$total_tc" -gt 0 ]; then
    cg_share=$(echo "scale=2; $total_cg * 100 / $total_tc" | bc 2>/dev/null || echo "0")
fi

summary=$(jq -n \
    --argjson total "$processed" \
    --argjson with_cg "$sessions_with_cg" \
    --argjson total_tc "$total_tc" \
    --argjson cg_count "$total_cg" \
    --arg adoption "$adoption_pct" \
    --arg share "$cg_share" \
    '{
        total_sessions: $total,
        sessions_with_codegraph: $with_cg,
        adoption_rate_pct: ($adoption | tonumber),
        total_tool_calls: $total_tc,
        codegraph_tool_calls: $cg_count,
        codegraph_share_pct: ($share | tonumber)
    }')

# -- Write output -------------------------------------------------------------
jq -n \
    --argjson summary "$summary" \
    --argjson by_tool "$tool_accum" \
    --argjson per_session "$per_session_json" \
    '{summary: $summary, by_tool: $by_tool, per_session: $per_session}' \
    > "$OUTPUT_DIR/agent.json"

echo ""
echo "=== Summary ==="
echo "Processed sessions: $processed"
echo "Skipped (no messages): $skipped"
echo "Sessions with codegraph: $sessions_with_cg"
echo "Adoption rate: ${adoption_pct}%"
echo "Total tool calls: $total_tc"
echo "Codegraph tool calls: $total_cg"
echo "Codegraph share: ${cg_share}%"
echo "=== Done ==="
echo "Results: $OUTPUT_DIR/agent.json"
