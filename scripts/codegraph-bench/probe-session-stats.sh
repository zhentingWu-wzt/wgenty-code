#!/usr/bin/env bash
set -euo pipefail
# probe-session-stats.sh
# Stat all session JSONs in ~/.wgenty-code/sessions/ for codegraph tool usage

SESSIONS_DIR="${SESSIONS_DIR:-$HOME/.wgenty-code/sessions}"
TOTAL=0
WITH_CODEGRAPH=0
TOTAL_TOOLCALLS=0
CODEGRAPH_TOOLCALLS=0

for f in "$SESSIONS_DIR"/*.json; do
  [ -f "$f" ] || continue
  TOTAL=$((TOTAL + 1))

  # Check if session has messages field
  has_msgs=$(jq 'has("messages")' "$f" 2>/dev/null || echo "false")
  if [ "$has_msgs" != "true" ]; then continue; fi

  # Count codegraph tool calls and all tool calls
  cg_calls=$(jq '[.messages[]?.tool_calls[]? | select(.function.name | test("codegraph"))] | length' "$f" 2>/dev/null || echo "0")
  all_calls=$(jq '[.messages[]?.tool_calls[]?] | length' "$f" 2>/dev/null || echo "0")

  TOTAL_TOOLCALLS=$((TOTAL_TOOLCALLS + all_calls))
  CODEGRAPH_TOOLCALLS=$((CODEGRAPH_TOOLCALLS + cg_calls))

  if [ "$cg_calls" -gt 0 ]; then
    WITH_CODEGRAPH=$((WITH_CODEGRAPH + 1))
  fi
done

echo "=== Session Statistics ==="
echo "Total sessions: $TOTAL"
echo "Sessions with codegraph calls: $WITH_CODEGRAPH"
echo "Codegraph adoption rate: $(( WITH_CODEGRAPH * 100 / (TOTAL > 0 ? TOTAL : 1) ))%"
echo "Total tool calls (all sessions): $TOTAL_TOOLCALLS"
echo "Codegraph tool calls: $CODEGRAPH_TOOLCALLS"
echo "Codegraph share of tool calls: $(( CODEGRAPH_TOOLCALLS * 100 / (TOTAL_TOOLCALLS > 0 ? TOTAL_TOOLCALLS : 1) ))%"
