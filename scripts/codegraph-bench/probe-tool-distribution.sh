#!/usr/bin/env bash
# Deep analysis: codegraph tool function name distribution
set -euo pipefail

SESSIONS_DIR="${HOME}/.wgenty-code/sessions"
echo "=== Codegraph Tool Distribution ==="
echo ""

# Get all codegraph tool call function names
for f in "$SESSIONS_DIR"/*.json; do
  [ -f "$f" ] || continue
  jq -r '[.messages[]?.tool_calls[]? | select(.function.name | test("codegraph")) | .function.name] | .[]' "$f" 2>/dev/null
done | sort | uniq -c | sort -rn

echo ""
echo "=== Sessions with codegraph calls ==="
echo "Session IDs:"
for f in "$SESSIONS_DIR"/*.json; do
  [ -f "$f" ] || continue
  cg_count=$(jq '[.messages[]?.tool_calls[]? | select(.function.name | test("codegraph"))] | length' "$f" 2>/dev/null || echo "0")
  if [ "$cg_count" -gt 0 ]; then
    sid=$(jq -r '.id // "unknown"' "$f" 2>/dev/null)
    echo "  $sid : $cg_count codegraph call(s)"
  fi
done

echo ""
echo "=== Sessions WITHOUT messages field (schema inconsistencies) ==="
for f in "$SESSIONS_DIR"/*.json; do
  has_msgs=$(jq 'has("messages")' "$f" 2>/dev/null || echo "parse_error")
  if [ "$has_msgs" != "true" ]; then
    sid=$(jq -r '.id // "unknown"' "$f" 2>/dev/null)
    echo "  $f (id: $sid)"
  fi
done
