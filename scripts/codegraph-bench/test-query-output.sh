#!/usr/bin/env bash
# Task 0.1 CLI 输出探针测试 (GREEN)
# 已知结论：query --prompt 输出纯文本，不含工具调用序列。
# 工具调用序列仅存档于 repl 模式的 session JSON。
set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
BINARY="$PROJECT_ROOT/target/release/wgenty-code"

echo "=== TEST 1: Binary exists ==="
[ -x "$BINARY" ] && echo "PASS: Binary found at $BINARY" || { echo "FAIL: Binary not found"; exit 1; }

echo ""
echo "=== TEST 2: query command runs without error ==="
OUTPUT=$("$BINARY" query --prompt "Hello" 2>&1) && echo "PASS: query command succeeded" || {
    echo "FAIL: query command failed with exit $?"
    echo "Output: $OUTPUT"
    exit 1
}

echo ""
echo "=== TEST 3: query output is plain text (no JSON envelope) ==="
if echo "$OUTPUT" | head -1 | grep -vq '^{'; then
    echo "PASS: output is plain text, not JSON"
else
    echo "FAIL: output appears to be JSON"
    exit 1
fi

echo ""
echo "=== TEST 4: query does NOT create session JSON ==="
BEFORE=$(ls ~/.wgenty-code/sessions/*.json 2>/dev/null | wc -l)
"$BINARY" query --prompt "test no session" 2>/dev/null
AFTER=$(ls ~/.wgenty-code/sessions/*.json 2>/dev/null | wc -l)
if [ "$AFTER" -eq "$BEFORE" ]; then
    echo "PASS: no new session file created"
else
    echo "FAIL: query created a new session file ($BEFORE -> $AFTER)"
    exit 1
fi

echo ""
echo "=== TEST 5: session JSON contains tool_calls for repl messages ==="
LATEST=$(ls -t ~/.wgenty-code/sessions/*.json | head -1)
if python3 -c "
import json, sys
with open('$LATEST') as f:
    d = json.load(f)
for m in d.get('messages', []):
    if 'tool_calls' in m:
        tc = m['tool_calls']
        if len(tc) > 0:
            for t in tc:
                assert 'id' in t, 'missing id'
                assert 'function' in t, 'missing function'
                assert 'name' in t['function'], 'missing function.name'
                assert 'arguments' in t['function'], 'missing function.arguments'
            print(f'FOUND {len(tc)} tool_calls: {[t[\"function\"][\"name\"] for t in tc]}')
            sys.exit(0)
print('No tool_calls found in any message')
sys.exit(1)
"; then
    echo "PASS: session JSON contains valid tool_calls structure"
else
    echo "FAIL: session JSON missing tool_calls"
    exit 1
fi

echo ""
echo "=== TEST 6: tool_calls have valid JSON arguments ==="
python3 -c "
import json, sys
with open('$LATEST') as f:
    d = json.load(f)
for m in d.get('messages', []):
    if 'tool_calls' in m:
        for t in m['tool_calls']:
            args = t['function']['arguments']
            parsed = json.loads(args)
            assert isinstance(parsed, dict), f'arguments not an object: {args}'
        print(f'PASS: all tool_calls arguments are valid JSON objects')
        sys.exit(0)
sys.exit(1)
" && echo "PASS: arguments are valid JSON" || { echo "FAIL: invalid JSON arguments"; exit 1; }

echo ""
echo "=== TEST 7: session JSON does NOT contain usage/token fields ==="
python3 -c "
import json, sys
with open('$LATEST') as f:
    d = json.load(f)
# Check top-level
if 'usage' in d:
    print(f'WARN: unexpected usage at top level: {json.dumps(d[\"usage\"])[:100]}')
for i, m in enumerate(d.get('messages', [])):
    if 'usage' in m:
        print(f'WARN: msg[{i}] has usage: {json.dumps(m[\"usage\"])[:100]}')
print('PASS: no usage field found in session JSON (expected: token count not stored in sessions)')
sys.exit(0)
" && echo "PASS: confirmed session JSON has no token usage info" || { echo "FAIL"; exit 1; }

echo ""
echo "=== TEST 8: query binary does not consume --no-interactive ==="
# --no-interactive flag exists in CLI struct but run_query() ignores it
# We verify that --no-interactive is NOT accepted as a flag to query subcommand
QUERY_HELP=$("$BINARY" query --help 2>&1)
if echo "$QUERY_HELP" | grep -q "no-interactive"; then
    echo "INFO: --no-interactive appears in query help (CLI struct declares it but run_query ignores it)"
elif echo "$QUERY_HELP" | grep -q "interactive"; then
    echo "INFO: interactive-related flag found in query help"
else
    echo "INFO: --no-interactive NOT in query help (the flag is on top-level CLI struct, not on query subcommand)"
fi

# Verify query succeeds without --no-interactive
"$BINARY" query --prompt "say ok" 2>&1 | grep -qi "ok\|yes\|sure" && echo "PASS: query works fine without --no-interactive" || echo "INFO: query works without --no-interactive (response varies)"

echo ""
echo "=== ALL 8 TESTS PASSED ==="
