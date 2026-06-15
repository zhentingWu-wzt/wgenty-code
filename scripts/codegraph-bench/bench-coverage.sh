#!/usr/bin/env bash
# bench-coverage.sh — 覆盖率基线测量脚本
#
# 测量 codegraph index 对目标 Rust 项目的覆盖程度：
#   1. 文件覆盖率（.rs 文件总数 vs 已索引文件数）
#   2. 符号与关系统计（SymbolKind 分组 / RelKind 分组）
#   3. 解析失败统计（parse 失败数 + 原因分布 top 3）
#
# 输出: $OUTPUT_DIR/coverage.json
# 依赖: lib/json-helpers.sh, jq, sqlite3 (3.33+ with -json)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/json-helpers.sh"

# =====================================================================
# Usage & Argument Parsing
# =====================================================================

usage() {
  cat <<'EOF'
Usage: bench-coverage.sh [OPTIONS]

Options:
  --target <path>    Target Rust project (default: .)
  --output <path>    Output directory (default: results/<ts>/)
  --wgenty <path>    Path to wgenty-code binary (auto-detect if omitted)
  --help             Show this help
EOF
  exit 0
}

TARGET="."
OUTPUT_DIR=""
WGENTY_BIN=""
RUN_TIMESTAMP=$(date +%Y%m%d-%H%M%S)

while [[ $# -gt 0 ]]; do
  case "$1" in
    --target) TARGET="$2"; shift 2 ;;
    --output) OUTPUT_DIR="$2"; shift 2 ;;
    --wgenty) WGENTY_BIN="$2"; shift 2 ;;
    --help) usage ;;
    *) echo "ERROR: Unknown option: $1" >&2; usage ;;
  esac
done

# ---- Default output dir ----
if [ -z "$OUTPUT_DIR" ]; then
  OUTPUT_DIR="results/$RUN_TIMESTAMP"
fi

# ---- Validate wgenty binary ----
if [ -z "$WGENTY_BIN" ]; then
  if [ -f "$SCRIPT_DIR/../../target/release/wgenty-code" ]; then
    WGENTY_BIN="$SCRIPT_DIR/../../target/release/wgenty-code"
  elif command -v wgenty-code &>/dev/null; then
    WGENTY_BIN="wgenty-code"
  else
    echo "ERROR: --wgenty is required (no auto-detect found)" >&2
    exit 1
  fi
fi

if [ ! -f "$WGENTY_BIN" ] && ! command -v "$WGENTY_BIN" &>/dev/null; then
  echo "ERROR: wgenty-code binary not found: $WGENTY_BIN" >&2
  exit 1
fi

# Resolve target to absolute path (ensures path comparison consistency)
TARGET="$(cd "$TARGET" 2>/dev/null && pwd)" || {
  echo "ERROR: target directory not found: $TARGET" >&2
  exit 1
}

# ---- Check dependencies ----
if ! command -v sqlite3 &>/dev/null; then
  echo "ERROR: sqlite3 is required" >&2
  exit 1
fi
if ! sqlite3 -json :memory: "SELECT 1 AS t" &>/dev/null; then
  echo "ERROR: sqlite3 does not support -json flag (need version 3.33+)" >&2
  exit 1
fi
if ! command -v jq &>/dev/null; then
  echo "ERROR: jq is required" >&2
  exit 1
fi

mkdir -p "$OUTPUT_DIR"

echo "=== bench-coverage.sh ==="
echo "Target: $TARGET"
echo "Output: $OUTPUT_DIR"
echo "Binary: $WGENTY_BIN"

# =====================================================================
# 0. Fresh index with stderr capture
# =====================================================================
CODEGRAPH_DIR="$TARGET/.codegraph"

if [ -d "$CODEGRAPH_DIR" ]; then
  echo "[setup] removing existing .codegraph/ for fresh measurement..."
  rm -rf "$CODEGRAPH_DIR"
fi

echo "[setup] running fresh index (stderr captured for parse failure analysis)..."
index_stderr=$("$WGENTY_BIN" codegraph index --target "$TARGET" 2>&1 >/dev/null) || true

INDEX_DB="$CODEGRAPH_DIR/index.db"
if [ ! -f "$INDEX_DB" ]; then
  echo "ERROR: index.db was not created after indexing (check --target path)" >&2
  exit 1
fi
echo "[setup] index.db created ($(wc -c < "$INDEX_DB" | tr -d ' ') bytes)"

echo "[setup] probing database tables..."
tables=$(sqlite3 "$INDEX_DB" ".tables" 2>/dev/null)
echo "  tables: $tables"

# =====================================================================
# 1. File Coverage (3.1)
# =====================================================================
echo ""
echo "--- File Coverage ---"

total_rs=$(find "$TARGET" -name '*.rs' -type f \
  -not -path '*/target/*' -not -path '*/.codegraph/*' 2>/dev/null | wc -l | tr -d ' ')
echo "  total .rs files: $total_rs"

indexed_files=$(sqlite3 "$INDEX_DB" "SELECT COUNT(*) FROM files" 2>/dev/null || echo 0)
echo "  indexed files: $indexed_files"

if [ "$total_rs" -gt 0 ]; then
  coverage_pct=$(awk "BEGIN { printf \"%.1f\", 100 * $indexed_files / $total_rs }")
else
  coverage_pct=0
fi
echo "  coverage: ${coverage_pct}%"

# Top 10 unindexed files — use diff approach (2 SQL queries, not N)
all_rs_list=$(mktemp /tmp/cg_all_rs.XXXXXX)
indexed_list=$(mktemp /tmp/cg_indexed.XXXXXX)
trap 'rm -f "$all_rs_list" "$indexed_list"' EXIT

find "$TARGET" -name '*.rs' -type f \
  -not -path '*/target/*' -not -path '*/.codegraph/*' 2>/dev/null | \
  sed "s|^$TARGET/||" | sort > "$all_rs_list"
sqlite3 "$INDEX_DB" "SELECT path FROM files" 2>/dev/null | sort > "$indexed_list"

unindexed_top10=$(comm -23 "$all_rs_list" "$indexed_list" 2>/dev/null | head -10 | \
  jq -R -s 'split("\n") | map(select(length > 0))')

echo "  unindexed top 10: $(echo "$unindexed_top10" | jq -c '.')"

# =====================================================================
# 2. Symbol & Relationship Statistics (3.2)
# =====================================================================
echo ""
echo "--- Symbol & Relationship Statistics ---"

total_symbols=$(sqlite3 "$INDEX_DB" "SELECT COUNT(*) FROM symbols" 2>/dev/null || echo 0)
symbols_json=$(sqlite3 -json "$INDEX_DB" \
  "SELECT kind, COUNT(*) AS count FROM symbols GROUP BY kind ORDER BY count DESC" \
  2>/dev/null) || symbols_json=""
symbols_json="${symbols_json:-[]}"
echo "  total symbols: $total_symbols"
echo "  symbols by kind: $(echo "$symbols_json" | jq -c '.')"

total_rels=$(sqlite3 "$INDEX_DB" "SELECT COUNT(*) FROM relationships" 2>/dev/null || echo 0)
rels_json=$(sqlite3 -json "$INDEX_DB" \
  "SELECT rel_kind AS kind, COUNT(*) AS count FROM relationships GROUP BY rel_kind ORDER BY count DESC" \
  2>/dev/null) || rels_json=""
rels_json="${rels_json:-[]}"
echo "  total relationships: $total_rels"
echo "  relationships by kind: $(echo "$rels_json" | jq -c '.')"

# =====================================================================
# 3. Parse Failure Statistics (3.3)
# =====================================================================
echo ""
echo "--- Parse Failure Statistics ---"

total_failures=$(echo "$index_stderr" | grep -c "Warning: failed to index" || true)

if [ "$total_failures" -gt 0 ]; then
  # Extract unique error messages, count by frequency, take top 3
  failure_reasons_json=$(
    echo "$index_stderr" | \
      grep "Warning: failed to index" | \
      sed -E 's/.*Warning: failed to index [^:]+: //' | \
      sort | uniq -c | sort -rn | head -3 | \
      jq -R -s '
        split("\n") | map(select(length > 0)) |
        map(capture("^\\s*(?<count>[0-9]+)\\s+(?<reason>.*)$")) |
        sort_by(.count | tonumber) | reverse |
        map(.reason)
      '
  )

  if [ "$total_rs" -gt 0 ]; then
    parse_fail_pct=$(awk "BEGIN { printf \"%.1f\", 100 * $total_failures / $total_rs }")
  else
    parse_fail_pct=0
  fi
else
  failure_reasons_json='[]'
  parse_fail_pct=0
fi

echo "  total parse failures: $total_failures"
echo "  failure rate: ${parse_fail_pct}%"
echo "  top 3 reasons: $(echo "$failure_reasons_json" | jq -c '.')"

# =====================================================================
# 4. Write Output JSON (3.4)
# =====================================================================
echo ""
echo "--- Writing $OUTPUT_DIR/coverage.json ---"

# Convert array of {kind, count} to flat dict {"kind": count, ...}
symbols_dict=$(echo "$symbols_json" | jq 'map({(.kind): .count}) | add // {}' 2>/dev/null || echo '{}')
rels_dict=$(echo "$rels_json" | jq 'map({(.kind): .count}) | add // {}' 2>/dev/null || echo '{}')

cat > "$OUTPUT_DIR/coverage.json" <<EOF
{
  "file_coverage": {
    "total_rs_files": $total_rs,
    "indexed_files": $indexed_files,
    "coverage_pct": $coverage_pct,
    "unindexed_top10": $unindexed_top10
  },
  "symbols": {
    "total": $total_symbols,
    "by_kind": $symbols_dict
  },
  "relationships": {
    "total": $total_rels,
    "by_kind": $rels_dict
  },
  "parse_failures": {
    "total": $total_failures,
    "pct": $parse_fail_pct,
    "top_reasons": $failure_reasons_json
  }
}
EOF

# Validate with jq
if jq '.' "$OUTPUT_DIR/coverage.json" > /dev/null 2>&1; then
  echo "  coverage.json: valid JSON ($(wc -c < "$OUTPUT_DIR/coverage.json" | tr -d ' ') bytes)"
  echo "  file coverage: $(jq -r '.file_coverage.coverage_pct' "$OUTPUT_DIR/coverage.json")%"
  echo "  symbols total: $(jq -r '.symbols.total' "$OUTPUT_DIR/coverage.json")"
  echo "  rels total:     $(jq -r '.relationships.total' "$OUTPUT_DIR/coverage.json")"
  echo "  parse failures: $(jq -r '.parse_failures.total' "$OUTPUT_DIR/coverage.json")"
else
  echo "  ERROR: coverage.json is not valid JSON" >&2
  exit 1
fi

echo ""
echo "=== bench-coverage.sh done ==="
