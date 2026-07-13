#!/usr/bin/env bash
# Assemble npm platform subpackages + main package from built artifacts.
#
# Usage: assemble.sh <version> <artifacts-dir> [out-dir]
#
# <artifacts-dir> is the directory produced by actions/download-artifact@v4
# when downloading all build artifacts: each subdirectory is an artifact
# named `binary-<rust-target>` containing the built binary.
#
# Output: <out-dir>/wgenty-code-<platform>-<arch>/  (5 platform packages)
#         <out-dir>/wgenty-code/                     (main launcher package)
set -euo pipefail

VERSION="${1:?missing version (arg 1)}"
ARTIFACTS_DIR="${2:?missing artifacts dir (arg 2)}"
OUT_DIR="${3:-npm-dist}"

# subpackage | rust-target | asset_name | binary_name
ENTRY=(
  "linux-x64|x86_64-unknown-linux-gnu|wgenty-code-linux-x86_64|wgenty-code"
  "linux-arm64|aarch64-unknown-linux-gnu|wgenty-code-linux-aarch64|wgenty-code"
  "darwin-x64|x86_64-apple-darwin|wgenty-code-macos-x86_64|wgenty-code"
  "darwin-arm64|aarch64-apple-darwin|wgenty-code-macos-aarch64|wgenty-code"
  "win32-x64|x86_64-pc-windows-msvc|wgenty-code-windows-x86_64.exe|wgenty-code.exe"
)

rm -rf "$OUT_DIR"
mkdir -p "$OUT_DIR"

for entry in "${ENTRY[@]}"; do
  IFS='|' read -r sub target asset bin <<< "$entry"
  src="$ARTIFACTS_DIR/binary-$target/$asset"
  if [[ ! -f "$src" ]]; then
    echo "::error::artifact not found: $src"
    echo "::error::available artifacts:"
    ls -1 "$ARTIFACTS_DIR" >&2 || true
    exit 1
  fi

  dir="$OUT_DIR/wgenty-code-$sub"
  mkdir -p "$dir"
  cp "npm/platforms/$sub/package.json" "$dir/package.json"
  cp "$src" "$dir/$bin"
  if [[ "$bin" != *.exe ]]; then
    chmod +x "$dir/$bin"
  fi
  (cd "$dir" && npm pkg set version="$VERSION" >/dev/null)
  echo "assembled $dir ($bin)"
done

# --- main launcher package -------------------------------------------------
main_dir="$OUT_DIR/wgenty-code"
mkdir -p "$main_dir/bin"
cp npm/wgenty-code/package.json "$main_dir/package.json"
cp npm/wgenty-code/bin/wgenty-code.js "$main_dir/bin/wgenty-code.js"
cp npm/wgenty-code/README.md "$main_dir/README.md"

# Stamp version into main package + all optionalDependencies values.
jq --arg v "$VERSION" \
  '.version=$v | .optionalDependencies = (.optionalDependencies | with_entries(.value = $v))' \
  "$main_dir/package.json" > "$main_dir/package.json.tmp"
mv "$main_dir/package.json.tmp" "$main_dir/package.json"

echo "assembled $main_dir"
echo "done: $(ls -1 "$OUT_DIR" | wc -l | tr -d ' ') packages in $OUT_DIR"
