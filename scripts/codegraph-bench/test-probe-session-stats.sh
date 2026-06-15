#!/usr/bin/env bash
# TDD test: verify probe-session-stats.sh against a known fixture
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
TEST_DIR="$SCRIPT_DIR/.test-cache"
rm -rf "$TEST_DIR"
mkdir -p "$TEST_DIR/sessions"

SESSIONS_DIR="$TEST_DIR/sessions"

# Fixture: 5 session JSONs with known codegraph call counts
# Session 1: 6 tool calls, 4 codegraph (codegraph_node x3, codegraph_explore x1)
cat > "$SESSIONS_DIR/session-1.json" << 'EOF'
{
  "id": "session-1",
  "messages": [
    {
      "role": "assistant",
      "tool_calls": [
        {"id": "call_1", "type": "function", "function": {"name": "codegraph_node", "arguments": "{}"}},
        {"id": "call_2", "type": "function", "function": {"name": "codegraph_explore", "arguments": "{}"}},
        {"id": "call_3", "type": "function", "function": {"name": "codegraph_node", "arguments": "{}"}},
        {"id": "call_4", "type": "function", "function": {"name": "grep", "arguments": "{}"}},
        {"id": "call_5", "type": "function", "function": {"name": "codegraph_node", "arguments": "{}"}},
        {"id": "call_6", "type": "function", "function": {"name": "Read", "arguments": "{}"}}
      ]
    }
  ]
}
EOF

# Session 2: 3 tool calls, 0 codegraph
cat > "$SESSIONS_DIR/session-2.json" << 'EOF'
{
  "id": "session-2",
  "messages": [
    {
      "role": "assistant",
      "tool_calls": [
        {"id": "call_a", "type": "function", "function": {"name": "grep", "arguments": "{}"}},
        {"id": "call_b", "type": "function", "function": {"name": "Read", "arguments": "{}"}},
        {"id": "call_c", "type": "function", "function": {"name": "Bash", "arguments": "{}"}}
      ]
    }
  ]
}
EOF

# Session 3: 0 tool calls (but has messages)
cat > "$SESSIONS_DIR/session-3.json" << 'EOF'
{
  "id": "session-3",
  "messages": [
    {"role": "user", "content": "hello"}
  ]
}
EOF

# Session 4: NO messages field (schema inconsistency edge case)
cat > "$SESSIONS_DIR/session-4.json" << 'EOF'
{
  "id": "session-4",
  "name": "legacy session"
}
EOF

# Session 5: 7 tool calls, 3 codegraph (all codegraph_node)
cat > "$SESSIONS_DIR/session-5.json" << 'EOF'
{
  "id": "session-5",
  "messages": [
    {
      "role": "assistant",
      "tool_calls": [
        {"id": "call_x", "type": "function", "function": {"name": "codegraph_node", "arguments": "{}"}},
        {"id": "call_y", "type": "function", "function": {"name": "codegraph_node", "arguments": "{}"}},
        {"id": "call_z", "type": "function", "function": {"name": "Read", "arguments": "{}"}},
        {"id": "call_w", "type": "function", "function": {"name": "Bash", "arguments": "{}"}},
        {"id": "call_v", "type": "function", "function": {"name": "codegraph_node", "arguments": "{}"}},
        {"id": "call_u", "type": "function", "function": {"name": "grep", "arguments": "{}"}},
        {"id": "call_t", "type": "function", "function": {"name": "Edit", "arguments": "{}"}}
      ]
    }
  ]
}
EOF

echo "=== Running tests against probe-session-stats.sh ==="

PROBE_SCRIPT="$SCRIPT_DIR/probe-session-stats.sh"
if [ ! -f "$PROBE_SCRIPT" ]; then
  echo "FAIL: probe-session-stats.sh does not exist yet (RED expected)"
  rm -rf "$TEST_DIR"
  exit 1
fi

# Run the probe script with SESSIONS_DIR pointing to our fixture
output=$(SESSIONS_DIR="$SESSIONS_DIR" /bin/bash "$PROBE_SCRIPT" 2>&1 || true)

# Parse output
total_sessions=$(echo "$output" | grep "Total sessions:" | awk '{print $NF}')
with_codegraph=$(echo "$output" | grep "Sessions with codegraph" | awk '{print $NF}')
total_toolcalls=$(echo "$output" | grep "Total tool calls" | awk '{print $NF}')
codegraph_calls=$(echo "$output" | grep "^Codegraph tool calls" | awk '{print $NF}')
adoption_rate=$(echo "$output" | grep "Codegraph adoption rate:" | grep -oE '[0-9]+%' | tr -d '%')
codegraph_share=$(echo "$output" | grep "Codegraph share of tool calls:" | grep -oE '[0-9]+%' | tr -d '%')

errors=0

# Expected values
# Total sessions: 5 (all 5 files)
# Sessions with codegraph: 2 (session-1 has 4, session-5 has 3)
# Total tool calls: 6 + 3 + 0 + 0 + 7 = 16
# Codegraph tool calls: 4 + 0 + 0 + 0 + 3 = 7
# Adoption rate: 2/5 = 40%
# Codegraph share: 7/16 = 43%

if [ "$total_sessions" != "5" ]; then
  echo "FAIL: Expected total sessions=5, got $total_sessions"
  errors=$((errors + 1))
else
  echo "PASS: Total sessions = 5"
fi

if [ "$with_codegraph" != "2" ]; then
  echo "FAIL: Expected sessions with codegraph=2, got $with_codegraph"
  errors=$((errors + 1))
else
  echo "PASS: Sessions with codegraph = 2"
fi

if [ "$total_toolcalls" != "16" ]; then
  echo "FAIL: Expected total tool calls=16, got $total_toolcalls"
  errors=$((errors + 1))
else
  echo "PASS: Total tool calls = 16"
fi

if [ "$codegraph_calls" != "7" ]; then
  echo "FAIL: Expected codegraph calls=7, got $codegraph_calls"
  errors=$((errors + 1))
else
  echo "PASS: Codegraph calls = 7"
fi

if [ "$adoption_rate" != "40" ]; then
  echo "FAIL: Expected adoption rate=40%, got ${adoption_rate}%"
  errors=$((errors + 1))
else
  echo "PASS: Adoption rate = 40%"
fi

if [ "$codegraph_share" != "43" ]; then
  echo "FAIL: Expected codegraph share=43%, got ${codegraph_share}%"
  errors=$((errors + 1))
else
  echo "PASS: Codegraph share = 43%"
fi

# Clean up
rm -rf "$TEST_DIR"

echo ""
if [ "$errors" -eq 0 ]; then
  echo "ALL TESTS PASSED"
else
  echo "$errors TEST(S) FAILED"
  exit 1
fi
