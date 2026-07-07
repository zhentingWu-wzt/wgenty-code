#!/usr/bin/env bash
# Wgenty Code — Installer (Linux / macOS)
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/zhentingWu-wzt/wgenty-code/main/scripts/install.sh | bash
#   bash scripts/install.sh                         # local
#   bash scripts/install.sh --version v0.1.0         # specific version
#   bash scripts/install.sh --build-from-source      # force source build
set -euo pipefail

REPO="zhentingWu-wzt/wgenty-code"
BINARY="wgenty-code"
DEFAULT_VERSION="${VERSION:-latest}"

# ── colour helpers ──────────────────────────────────────────────────────────

RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'
BLUE='\033[0;34m'; CYAN='\033[0;36m'; NC='\033[0m'

header()  { printf "${BLUE}══ %s ══${NC}\n" "$1"; }
ok()      { printf "${GREEN}✓${NC} %s\n" "$1"; }
warn()    { printf "${YELLOW}⚠${NC} %s\n" "$1"; }
err()     { printf "${RED}✗${NC} %s\n" "$1"; }
info()    { printf "${CYAN}›${NC} %s\n" "$1"; }

# ── args ────────────────────────────────────────────────────────────────────

VERSION="$DEFAULT_VERSION"
BUILD_FROM_SOURCE=false
INSTALL_DIR=""

while [[ $# -gt 0 ]]; do
    case "$1" in
        --version)          VERSION="$2"; shift 2 ;;
        --build-from-source) BUILD_FROM_SOURCE=true; shift ;;
        --install-dir)      INSTALL_DIR="$2"; shift 2 ;;
        *) err "Unknown arg: $1"; exit 1 ;;
    esac
done

# ── detect os / arch ────────────────────────────────────────────────────────

detect_platform() {
    local os arch
    case "$(uname -s)" in
        Linux)  os="linux" ;;
        Darwin) os="macos" ;;
        *)      err "Unsupported OS: $(uname -s)"; exit 1 ;;
    esac
    case "$(uname -m)" in
        x86_64|amd64) arch="x86_64" ;;
        aarch64|arm64) arch="aarch64" ;;
        *) err "Unsupported arch: $(uname -m)"; exit 1 ;;
    esac
    PLATFORM="${os}-${arch}"
    ASSET="${BINARY}-${PLATFORM}"
    ok "Platform: ${PLATFORM}"
}

# ── pick install dir ────────────────────────────────────────────────────────

pick_install_dir() {
    if [[ -n "$INSTALL_DIR" ]]; then
        ok "Install dir (user-specified): ${INSTALL_DIR}"
        return
    fi
    local candidates=("$HOME/.local/bin" "$HOME/bin" "/usr/local/bin")
    for d in "${candidates[@]}"; do
        if [[ -d "$d" && -w "$d" ]]; then
            INSTALL_DIR="$d"
            break
        fi
    done
    if [[ -z "$INSTALL_DIR" ]]; then
        INSTALL_DIR="$HOME/.local/bin"
        mkdir -p "$INSTALL_DIR"
    fi
    ok "Install dir: ${INSTALL_DIR}"
}

# ── download release binary ──────────────────────────────────────────────────

download_release() {
    local url asset="$ASSET"
    if [[ "$VERSION" == "latest" ]]; then
        url="https://github.com/${REPO}/releases/latest/download/${asset}"
        info "Downloading latest release..."
    else
        url="https://github.com/${REPO}/releases/download/${VERSION}/${asset}"
        info "Downloading ${VERSION}..."
    fi

    mkdir -p "$INSTALL_DIR"
    if curl -fsSL -L -o "${INSTALL_DIR}/${BINARY}" "$url"; then
        chmod +x "${INSTALL_DIR}/${BINARY}"
        ok "Downloaded"
        return 0
    else
        warn "Download failed (${url})"
        return 1
    fi
}

# ── build from source ───────────────────────────────────────────────────────

build_from_source() {
    local script_dir repo_root
    script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
    repo_root="$(cd "${script_dir}/.." && pwd)"

    if [[ ! -f "${repo_root}/Cargo.toml" ]]; then
        err "Not a wgenty-code source tree: ${repo_root}"
        exit 1
    fi

    info "Building from source (this may take a few minutes)..."
    if ! command -v cargo &>/dev/null; then
        err "Rust is not installed. Visit https://rustup.rs/"
        exit 1
    fi

    (cd "$repo_root" && cargo build --release)
    cp "${repo_root}/target/release/${BINARY}" "${INSTALL_DIR}/${BINARY}"
    chmod +x "${INSTALL_DIR}/${BINARY}"
    ok "Built & installed from source"
}

# ── ensure PATH ─────────────────────────────────────────────────────────────

ensure_path() {
    if ! echo "$PATH" | tr ':' '\n' | grep -qxF "$INSTALL_DIR"; then
        warn "${INSTALL_DIR} is not in PATH"
        local rc=""
        case "$(basename "${SHELL:-/bin/bash}")" in
            zsh)  rc="$HOME/.zshrc" ;;
            bash) rc="$HOME/.bashrc" ;;
        esac
        if [[ -n "$rc" && -w "$rc" ]]; then
            echo "export PATH=\"${INSTALL_DIR}:\$PATH\"" >> "$rc"
            ok "Added to ${rc} (restart your shell to take effect)"
        else
            warn "Add this line to your shell config:"
            echo "  export PATH=\"${INSTALL_DIR}:\$PATH\""
        fi
    else
        ok "PATH includes ${INSTALL_DIR}"
    fi
}

# ── verify ──────────────────────────────────────────────────────────────────

verify() {
    header "Verify"
    if ! "${INSTALL_DIR}/${BINARY}" --version &>/dev/null; then
        err "Verification failed — binary does not run"
        exit 1
    fi
    ok "$("${INSTALL_DIR}/${BINARY}" --version)"
    echo ""
    info "Try: ${BINARY} repl"
    info "     ${BINARY} --help"
}

# ── main ────────────────────────────────────────────────────────────────────

cleanup() {
    if [[ $? -ne 0 ]]; then
        err "Installation aborted"
    fi
}
trap cleanup EXIT

header "Wgenty Code Installer"

detect_platform
pick_install_dir

if [[ "$BUILD_FROM_SOURCE" == true ]]; then
    build_from_source
elif ! download_release; then
    warn "Falling back to source build..."
    build_from_source
fi

ensure_path
verify

echo ""
ok "Done — wgenty-code is ready!"
