#!/usr/bin/env bash
# Build the wgenty-code desktop app (Tauri) with the daemon bundled.
#
# Steps:
#   1. Build the daemon CLI binary (release, host target).
#   2. Stage it into desktop/src-tauri/binaries/ under the Tauri externalBin
#      naming convention: wgenty-code-<target-triple>[.exe].
#   3. Build the web frontend (web/dist).
#   4. cargo tauri build (uses externalBin from tauri.conf.json).
#
# Output: desktop/src-tauri/target/release/bundle/
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$REPO_ROOT"

TRIPLE="$(rustc -vV | sed -n 's/^host: //p')"
SUFFIX=""
case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*) SUFFIX=".exe" ;;
esac

echo ">> [1/4] Building daemon (release, $TRIPLE)..."
cargo build --release

echo ">> [2/4] Staging daemon for Tauri externalBin..."
mkdir -p desktop/src-tauri/binaries
cp "target/release/wgenty-code$SUFFIX" "desktop/src-tauri/binaries/wgenty-code-$TRIPLE$SUFFIX"

echo ">> [3/4] Building web frontend..."
(cd web && npm run build)

echo ">> [4/4] Building Tauri app..."
(cd desktop/src-tauri && cargo tauri build)

echo ">> Done. Bundles:"
ls -1 desktop/src-tauri/target/release/bundle/
