#!/usr/bin/env bash
# 采集测量环境指纹，统一输出到 env.json
set -euo pipefail

fingerprint_env() {
  local output_dir="${1:-.}"
  local target="${2:-.}"
  local timestamp
  timestamp=$(date -u +"%Y-%m-%dT%H:%M:%SZ")

  local wgenty_bin="${WGENTY_BIN:-}"
  if [ -z "$wgenty_bin" ]; then
    local script_dir
    script_dir="$(cd "$(dirname "$0")/../.." && pwd)"
    if [ -f "$script_dir/target/release/wgenty-code" ]; then
      wgenty_bin="$script_dir/target/release/wgenty-code"
    elif command -v wgenty-code &>/dev/null; then
      wgenty_bin="wgenty-code"
    fi
  fi

  local os_name cpu_count wgenty_version commit_hash
  os_name="$(uname -s) $(uname -m)"
  cpu_count=$(getconf _NPROCESSORS_ONLN 2>/dev/null || sysctl -n hw.ncpu 2>/dev/null || echo "unknown")

  if [ -n "$wgenty_bin" ] && [ -x "$wgenty_bin" ]; then
    wgenty_version=$("$wgenty_bin" --version 2>/dev/null || echo "unknown")
  else
    wgenty_version="unknown (binary not found)"
  fi

  pushd "$target" > /dev/null
  commit_hash=$(git rev-parse HEAD 2>/dev/null || echo "not-a-git-repo")
  popd > /dev/null

  mkdir -p "$output_dir"
  cat > "$output_dir/env.json" <<EOF
{
  "timestamp": "$timestamp",
  "os": "$os_name",
  "cpu_count": "$cpu_count",
  "wgenty_version": "$wgenty_version",
  "target_commit": "$commit_hash",
  "target_path": "$(cd "$target" && pwd)",
  "wgenty_bin": "$wgenty_bin"
}
EOF
  echo "[fingerprint] env.json -> $output_dir/env.json"
}
