#!/usr/bin/env bash
# Full tool distribution across all sessions (compatible with bash 3.2)
set -euo pipefail

SESSIONS_DIR="${HOME}/.wgenty-code/sessions"

echo "=== Full Tool Distribution (all sessions) ==="
echo ""

# Write all tool call function names, one per line, then sort|uniq -c
for f in "$SESSIONS_DIR"/*.json; do
  [ -f "$f" ] || continue
  jq -r '.messages[]?.tool_calls[]?.function.name // empty' "$f" 2>/dev/null
done | sort | uniq -c | sort -rn | head -30
