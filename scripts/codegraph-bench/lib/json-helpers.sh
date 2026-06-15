#!/usr/bin/env bash
set -euo pipefail

compute_percentiles() {
  local arr="$1"
  echo "$arr" | jq '{
    samples: .,
    count: length,
    median: (sort | if length % 2 == 1 then .[length/2 | floor] else (.[length/2] + .[length/2 - 1]) / 2 end),
    p95: (sort | .[ (length * 0.95 | ceil) - 1 ])
  }'
}

numbers_to_json() {
  local nums="$1"
  echo "$nums" | tr ',' '\n' | jq -R -s 'split("\n") | map(select(length > 0) | tonumber)'
}
