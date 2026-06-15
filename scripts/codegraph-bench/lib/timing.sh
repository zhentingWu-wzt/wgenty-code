#!/usr/bin/env bash
set -euo pipefail

time_cmd() {
  local label="$1"
  shift
  local start end elapsed_ms
  start=$(date +%s%N)
  "$@"
  local rc=$?
  end=$(date +%s%N)
  elapsed_ms=$(( (end - start) / 1000000 ))
  echo "[timing] $label: ${elapsed_ms}ms (exit=$rc)"
  return $rc
}
