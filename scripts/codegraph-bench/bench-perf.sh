#!/usr/bin/env bash
# bench-perf.sh — 性能基线测量脚本
#
# 测量 codegraph index 的以下基线：
#   1. 全量索引耗时（多次运行，每次清空 .codegraph/）
#   2. 增量索引耗时（touch 1/10/100 个 .rs 文件，各 3+ 次）
#   3. 索引体积（index.db 字节 / 源码 .rs 字节比）
#
# 输出: $OUTPUT_DIR/perf.json
# 依赖: lib/timing.sh, lib/json-helpers.sh, jq
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
source "$SCRIPT_DIR/lib/timing.sh"
source "$SCRIPT_DIR/lib/json-helpers.sh"

# =====================================================================
# Usage & Argument Parsing
# =====================================================================

usage() {
  cat <<'EOF'
Usage: bench-perf.sh [OPTIONS]

Options:
  --target <path>    Target Rust project (default: .)
  --output <path>    Output directory (required)
  --wgenty <path>    Path to wgenty-code binary (required)
  --repeats <n>      Full-index repeats (default: 5)
  --help             Show this help
EOF
  exit 0
}

TARGET="."
OUTPUT_DIR=""
WGENTY_BIN=""
REPEATS=5

while [[ $# -gt 0 ]]; do
  case "$1" in
    --target) TARGET="$2"; shift 2 ;;
    --output) OUTPUT_DIR="$2"; shift 2 ;;
    --wgenty) WGENTY_BIN="$2"; shift 2 ;;
    --repeats) REPEATS="$2"; shift 2 ;;
    --help) usage ;;
    *) echo "Unknown: $1" >&2; usage ;;
  esac
done

# ---- Validation ----
if [ -z "$OUTPUT_DIR" ]; then
  echo "ERROR: --output is required" >&2
  exit 1
fi

if [ -z "$WGENTY_BIN" ]; then
  # Auto-detect fallback
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

if [ ! -d "$TARGET" ]; then
  echo "ERROR: target directory not found: $TARGET" >&2
  exit 1
fi

if ! command -v jq &>/dev/null; then
  echo "ERROR: jq is required" >&2
  exit 1
fi

mkdir -p "$OUTPUT_DIR"

echo "=== bench-perf.sh ==="
echo "Target: $TARGET"
echo "Output: $OUTPUT_DIR"
echo "Binary: $WGENTY_BIN"
echo "Repeats: $REPEATS"

# =====================================================================
# Helper Functions
# =====================================================================

# clean_codegraph <dir>
#   递归删除目标目录下的 .codegraph/
clean_codegraph() {
  local d="$1"
  if [ -d "$d/.codegraph" ]; then
    rm -rf "$d/.codegraph"
    echo "[clean] removed $d/.codegraph/"
  else
    echo "[clean] .codegraph/ already absent"
  fi
}

# extract_ms
#   从 stdin 解析 time_cmd 输出的毫秒数
#   匹配格式: [timing] label: 1234ms (exit=0)
extract_ms() {
  sed -nE 's/.*\[timing\].*: ([0-9]+)ms.*/\1/p'
}

# ms_to_sec <ms>
#   将毫秒值转换为秒（浮点，三位小数）
ms_to_sec() {
  local ms="$1"
  awk "BEGIN { printf \"%.3f\", $ms / 1000 }"
}

# count_rs_files <dir>
#   返回目录中 .rs 文件的总数（排除 target/）
count_rs_files() {
  local dir="$1"
  find "$dir" -name '*.rs' -type f -not -path '*/target/*' 2>/dev/null | wc -l | tr -d ' '
}

# pick_random_files <dir> <count>
#   在 dir 中随机选取 count 个 .rs 文件（排除 target/）
#   使用 POSIX 兼容的 awk+sort 替代 GNU shuf
pick_random_files() {
  local dir="$1"
  local count="$2"
  find "$dir" -name '*.rs' -type f -not -path '*/target/*' 2>/dev/null | \
    awk 'BEGIN{srand()} {print rand(), $0}' | sort -n | head -n "$count" | \
    awk '{print $2}'
}

# =====================================================================
# 1. Full Index Timing
# =====================================================================
echo ""
echo "--- Full Index Timing ($REPEATS runs) ---"

full_samples_ms=()

for i in $(seq 1 "$REPEATS"); do
  clean_codegraph "$TARGET"
  echo "[full run $i] indexing..."

  output=$(cd "$TARGET" && time_cmd "full_$i" "$WGENTY_BIN" codegraph index 2>&1)
  ms=$(echo "$output" | extract_ms | tail -1)

  if [ -z "$ms" ]; then
    echo "  WARNING: could not extract timing for run $i" >&2
    continue
  fi

  full_samples_ms+=("$ms")
  echo "  run $i: ${ms}ms"
done

if [ ${#full_samples_ms[@]} -eq 0 ]; then
  echo "ERROR: no successful full index runs" >&2
  exit 1
fi

# Convert samples from ms to seconds
full_samples_sec=()
for ms in "${full_samples_ms[@]}"; do
  full_samples_sec+=("$(ms_to_sec "$ms")")
done

full_json=$(IFS=','; numbers_to_json "${full_samples_sec[*]}")
full_perf=$(compute_percentiles "$full_json")
echo "  full index result: $(echo "$full_perf" | jq -c '{samples, median, p95}')"

# =====================================================================
# 2. Incremental Index Timing
# =====================================================================
echo ""
echo "--- Incremental Index Timing ---"

# Build a fresh full index as baseline for all incremental tests
clean_codegraph "$TARGET"
echo "[setup] building baseline full index..."
if ! (cd "$TARGET" && "$WGENTY_BIN" codegraph index) > /dev/null 2>&1; then
  echo "ERROR: baseline full index failed" >&2
  exit 1
fi
echo "[setup] baseline full index done."

INCREMENTAL_REPEATS=3

# Check available .rs files
TOTAL_RS=$(count_rs_files "$TARGET")
echo "[setup] found $TOTAL_RS .rs files in target"

run_incremental_bench() {
  local label="$1"
  local count="$2"
  local samples_ms=()

  if [ "$TOTAL_RS" -lt "$count" ]; then
    echo "  SKIP: only $TOTAL_RS .rs files available, need $count"
    echo "[]"
    return
  fi

  echo "  Touching $count .rs file(s) per run..."

  for i in $(seq 1 "$INCREMENTAL_REPEATS"); do
    # Select random files to touch
    local files=()
    while IFS= read -r f; do
      files+=("$f")
    done < <(pick_random_files "$TARGET" "$count")

    if [ ${#files[@]} -eq 0 ]; then
      echo "    WARNING: no .rs files selected" >&2
      continue
    fi

    # Touch (modify mtime without changing content)
    for f in "${files[@]}"; do
      touch "$f"
    done

    # Run incremental index and capture timing
    output=$(cd "$TARGET" && time_cmd "incr_${label}_${i}" "$WGENTY_BIN" codegraph index 2>&1)
    ms=$(echo "$output" | extract_ms | tail -1)

    if [ -z "$ms" ]; then
      echo "    WARNING: no timing for ${label} run ${i}" >&2
      continue
    fi

    samples_ms+=("$ms")
    echo "    run ${i}: ${ms}ms"
  done

  # Convert to seconds and build JSON array
  local secs=()
  for ms in "${samples_ms[@]}"; do
    secs+=("$(ms_to_sec "$ms")")
  done

  if [ ${#secs[@]} -eq 0 ]; then
    echo "[]"
    return
  fi

  local secs_csv
  secs_csv=$(IFS=','; echo "${secs[*]}")
  numbers_to_json "$secs_csv"
}

echo ""
echo "  [incremental] 1 file ..."
incremental_1_json=$(run_incremental_bench "1_file" 1)

echo ""
echo "  [incremental] 10 files ..."
incremental_10_json=$(run_incremental_bench "10_files" 10)

echo ""
echo "  [incremental] 100 files ..."
incremental_100_json=$(run_incremental_bench "100_files" 100)

# =====================================================================
# 3. Index Volume
# =====================================================================
echo ""
echo "--- Index Volume ---"

CODEGRAPH_DIR="$TARGET/.codegraph"
INDEX_DB=""
index_db_bytes=0

if [ -d "$CODEGRAPH_DIR" ]; then
  # Find primary database file (SQLite *.db or *.sqlite)
  INDEX_DB=$(find "$CODEGRAPH_DIR" -type f \( -name '*.db' -o -name '*.sqlite' \) 2>/dev/null | head -1)
fi

if [ -n "$INDEX_DB" ] && [ -f "$INDEX_DB" ]; then
  # macOS stat vs Linux stat
  if [ "$(uname)" = "Darwin" ]; then
    index_db_bytes=$(stat -f%z "$INDEX_DB" 2>/dev/null || echo 0)
  else
    index_db_bytes=$(stat -c%s "$INDEX_DB" 2>/dev/null || echo 0)
  fi
  echo "  index.db: $INDEX_DB ($index_db_bytes bytes)"
elif [ -d "$CODEGRAPH_DIR" ]; then
  # Fallback: measure entire .codegraph/ directory
  echo "  (no .db/.sqlite found, measuring full .codegraph/ directory)"
  index_db_bytes=$(du -sb "$CODEGRAPH_DIR" 2>/dev/null | cut -f1 || echo 0)
  echo "  .codegraph/ total: $index_db_bytes bytes"
else
  echo "  WARNING: .codegraph/ directory not found" >&2
fi

# Sum all .rs source file bytes (excluding target/)
source_rs_bytes=0
rs_file_count=0
while IFS= read -r -d '' f; do
  if [ "$(uname)" = "Darwin" ]; then
    sz=$(stat -f%z "$f" 2>/dev/null || echo 0)
  else
    sz=$(stat -c%s "$f" 2>/dev/null || echo 0)
  fi
  source_rs_bytes=$((source_rs_bytes + sz))
  rs_file_count=$((rs_file_count + 1))
done < <(find "$TARGET" -name '*.rs' -type f -not -path '*/target/*' -print0 2>/dev/null)

# Compute ratio
if [ "$index_db_bytes" -gt 0 ] && [ "$source_rs_bytes" -gt 0 ]; then
  ratio=$(awk "BEGIN { printf \"%.2f\", $index_db_bytes / $source_rs_bytes }")
else
  ratio=0
  echo "  WARNING: cannot compute ratio (index_db_bytes=$index_db_bytes, source_rs_bytes=$source_rs_bytes)" >&2
fi

echo "  source .rs: $rs_file_count files, $source_rs_bytes bytes"
echo "  index_db / source ratio: $ratio"

# =====================================================================
# 4. Write Output JSON
# =====================================================================
echo ""
echo "--- Writing $OUTPUT_DIR/perf.json ---"

# Ensure incremental JSONs have fallback values for empty results
incremental_1_json="${incremental_1_json:-[]}"
incremental_10_json="${incremental_10_json:-[]}"
incremental_100_json="${incremental_100_json:-[]}"

# Wrap samples into result objects with stats
incremental_1_perf=$(compute_percentiles "$incremental_1_json" 2>/dev/null || echo '{"samples":[]}')
incremental_10_perf=$(compute_percentiles "$incremental_10_json" 2>/dev/null || echo '{"samples":[]}')
incremental_100_perf=$(compute_percentiles "$incremental_100_json" 2>/dev/null || echo '{"samples":[]}')

cat > "$OUTPUT_DIR/perf.json" <<EOF
{
  "full_index": $full_perf,
  "incremental_index": {
    "1_file": $incremental_1_perf,
    "10_files": $incremental_10_perf,
    "100_files": $incremental_100_perf
  },
  "index_size": {
    "index_db_bytes": $index_db_bytes,
    "source_rs_bytes": $source_rs_bytes,
    "ratio": $ratio
  }
}
EOF

# Validate with jq
if jq '.' "$OUTPUT_DIR/perf.json" > /dev/null 2>&1; then
  echo "  perf.json: valid JSON ($(wc -c < "$OUTPUT_DIR/perf.json" | tr -d ' ') bytes)"
  echo "  full_index median: $(jq -r '.full_index.median' "$OUTPUT_DIR/perf.json")"
else
  echo "  ERROR: perf.json is not valid JSON" >&2
  exit 1
fi

echo ""
echo "=== bench-perf.sh done ==="
