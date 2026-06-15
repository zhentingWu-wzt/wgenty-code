#!/usr/bin/env bash
# TDD test: verify probe-ripgrep-index.txt exists with expected fields
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROBE_FILE="$SCRIPT_DIR/probe-ripgrep-index.txt"

echo "=== TEST 1: Probe file exists ==="
if [ -f "$PROBE_FILE" ]; then
    echo "PASS: probe-ripgrep-index.txt exists"
else
    echo "FAIL: probe-ripgrep-index.txt does not exist (RED expected - probe not yet run)"
    exit 1
fi

echo ""
echo "=== TEST 2: Probe file contains essential fields ==="
required_fields=(
    "Rust files:"
    "Index duration:"
    "Index DB size:"
    "Symbols:"
    "Relationships:"
    "Conclusion:"
)
missing=0
for field in "${required_fields[@]}"; do
    if grep -q "$field" "$PROBE_FILE"; then
        echo "PASS: Field '$field' present"
    else
        echo "FAIL: Field '$field' missing"
        missing=$((missing + 1))
    fi
done

if [ "$missing" -gt 0 ]; then
    echo "FAIL: $missing required fields missing from probe file"
    exit 1
fi

echo ""
echo "=== TEST 3: Rust files count is positive integer ==="
rust_files=$(grep "^Rust files:" "$PROBE_FILE" | awk '{print $NF}')
if [[ "$rust_files" =~ ^[0-9]+$ ]] && [ "$rust_files" -gt 0 ]; then
    echo "PASS: Rust files = $rust_files (positive integer)"
else
    echo "FAIL: Rust files count not a positive integer, got '$rust_files'"
    exit 1
fi

echo ""
echo "=== TEST 4: Index duration is non-empty ==="
duration=$(grep "^Index duration:" "$PROBE_FILE" | sed 's/^Index duration: //')
if [ -n "$duration" ]; then
    echo "PASS: Index duration recorded: $duration"
else
    echo "FAIL: Index duration is empty"
    exit 1
fi

echo ""
echo "=== TEST 5: DB size is non-negative ==="
db_size=$(grep "^Index DB size:" "$PROBE_FILE" | awk '{print $NF}')
if [[ "$db_size" =~ ^[0-9]+[BKMG]?$ ]] || [[ "$db_size" =~ ^[0-9]+(\.[0-9]+)?[BKMG]?$ ]]; then
    echo "PASS: DB size = $db_size"
else
    echo "FAIL: DB size not recognized, got '$db_size'"
    exit 1
fi

echo ""
echo "=== TEST 6: Symbols count is non-negative integer ==="
symbols=$(grep "^Symbols:" "$PROBE_FILE" | awk '{print $NF}')
if [[ "$symbols" =~ ^[0-9]+$ ]]; then
    echo "PASS: Symbols = $symbols"
else
    echo "FAIL: Symbols count not a number, got '$symbols'"
    exit 1
fi

echo ""
echo "=== TEST 7: Relationships count is non-negative integer ==="
rels=$(grep "^Relationships:" "$PROBE_FILE" | awk '{print $NF}')
if [[ "$rels" =~ ^[0-9]+$ ]]; then
    echo "PASS: Relationships = $rels"
else
    echo "FAIL: Relationships count not a number, got '$rels'"
    exit 1
fi

echo ""
echo "=== TEST 8: Conclusion field indicates suitability ==="
conclusion=$(grep "^Conclusion:" "$PROBE_FILE" | sed 's/^Conclusion: //')
if echo "$conclusion" | grep -qiE "suitable|yes|appropriate|good|viable|pass|recommended"; then
    echo "PASS: Conclusion recommends ripgrep as verification target"
elif echo "$conclusion" | grep -qiE "unsuitable|no|not appropriate|not recommended|fail"; then
    echo "FAIL: Conclusion does NOT recommend ripgrep as verification target: $conclusion"
    exit 1
else
    echo "INFO: Conclusion is neutral, treating as pass: $conclusion"
fi

echo ""
echo "=== ALL 8 TESTS PASSED ==="
