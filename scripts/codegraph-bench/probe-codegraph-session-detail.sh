#!/usr/bin/env bash
# Deep dive into the codegraph-using session
set -euo pipefail

SESSIONS_DIR="${HOME}/.wgenty-code/sessions"
F="$SESSIONS_DIR/37ab7bbd-4c8c-4040-be81-3106d3a12d0b.json"

echo "=== Codegraph-using session details ==="
echo "Session name: $(jq -r '.name // "(unnamed)"' "$F")"
echo "Created at: $(jq -r '.created_at' "$F")"
echo ""
echo "All tool call functions used:"
jq '[.messages[]?.tool_calls[]?.function.name] | unique' "$F"
echo ""
echo "Total tool calls in session:"
jq '[.messages[]?.tool_calls[]?] | length' "$F"
echo ""
echo "Total messages:"
jq '[.messages[]?] | length' "$F"
echo ""
echo "Message roles:"
jq '[.messages[]?.role] | unique' "$F"
