#!/usr/bin/env bash
# Wgenty Code — Unix/macOS installer
# Usage: curl -sSL https://raw.githubusercontent.com/zhentingWu-wzt/wgenty-code/master/install-unix.sh | bash

set -euo pipefail

REPO="https://github.com/zhentingWu-wzt/wgenty-code.git"
INSTALL_DIR="${HOME}/.wgenty-code/bin"
TMP_DIR=$(mktemp -d)

cleanup() {
    rm -rf "$TMP_DIR"
}
trap cleanup EXIT

echo "==> Cloning wgenty-code..."
git clone --depth 1 "$REPO" "$TMP_DIR/wgenty-code"
cd "$TMP_DIR/wgenty-code"

echo "==> Building wgenty-code (release)..."
cargo build --release 2>&1 | tail -5

echo "==> Installing to ${INSTALL_DIR}..."
mkdir -p "$INSTALL_DIR"
cp ./target/release/wgenty-code "$INSTALL_DIR/wgenty-code"
chmod +x "$INSTALL_DIR/wgenty-code"

# Add to PATH if not already present
SHELL_RC=""
case "$SHELL" in
    */zsh) SHELL_RC="$HOME/.zshrc" ;;
    */bash) SHELL_RC="$HOME/.bashrc" ;;
    *) SHELL_RC="$HOME/.profile" ;;
esac

if ! grep -q "$INSTALL_DIR" "$SHELL_RC" 2>/dev/null; then
    echo "export PATH=\"$INSTALL_DIR:\$PATH\"" >> "$SHELL_RC"
    echo "==> Added ${INSTALL_DIR} to PATH in ${SHELL_RC}"
fi

echo ""
echo "==> Installation complete!"
echo "    Run 'source ${SHELL_RC}' or restart your shell, then:"
echo "    wgenty-code --version"
echo ""
echo "    Set your API key: export ANTHROPIC_API_KEY=\"sk-ant-...\""
