#!/bin/bash
# DJ USB Tkit — macOS Build Setup
# Installs all prerequisites and builds the app.
# Run from anywhere — the script finds the project root relative to itself.
#
# Usage:
#   chmod +x scripts/macos-build-setup.sh
#   ./scripts/macos-build-setup.sh

set -e

echo ""
echo "=== DJ USB Tkit macOS Build Setup ==="

# ─── 1. Xcode Command Line Tools ────────────────────────────────────────────
echo ""
echo "[1/4] Checking Xcode Command Line Tools..."
if xcode-select -p &>/dev/null; then
    echo "  Already installed: $(xcode-select -p)"
else
    echo "  Installing Xcode Command Line Tools..."
    echo "  A system dialog will appear — click 'Install' and wait for it to finish."
    xcode-select --install
    # Wait until the tools are actually installed
    echo "  Waiting for installation to complete (this can take several minutes)..."
    until xcode-select -p &>/dev/null; do sleep 5; done
    echo "  Done."
fi

# ─── 2. Rust ────────────────────────────────────────────────────────────────
echo ""
echo "[2/4] Checking Rust..."
# Source cargo env if it exists (may be from a previous partial install)
[ -f "$HOME/.cargo/env" ] && source "$HOME/.cargo/env"

# Verify rustc actually works (not just that the binary exists)
RUST_OK=""
if command -v rustc &>/dev/null; then
    if rustc --version &>/dev/null; then
        RUST_OK="$(rustc --version)"
    fi
fi

if [ -n "$RUST_OK" ]; then
    echo "  Already installed: $RUST_OK"
else
    echo "  Installing Rust via rustup (this downloads ~300MB)..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
    source "$HOME/.cargo/env"
    # Verify
    if ! rustc --version &>/dev/null; then
        echo "  ERROR: rustc not working after install." >&2
        exit 1
    fi
    echo "  Installed: $(rustc --version)"
fi

# Add native target only (skip cross-compilation targets to save bandwidth)
echo "  Ensuring native target..."
ARCH=$(uname -m)
if [ "$ARCH" = "arm64" ]; then
    rustup target add aarch64-apple-darwin 2>/dev/null || true
else
    rustup target add x86_64-apple-darwin 2>/dev/null || true
fi

# ─── 3. Node.js ─────────────────────────────────────────────────────────────
echo ""
echo "[3/4] Checking Node.js..."
if command -v node &>/dev/null; then
    echo "  Already installed: $(node --version)"
else
    # Install Node locally to ~/.local/nodejs to avoid needing sudo
    NODE_VER="v24.14.1"
    ARCH=$(uname -m)
    if [ "$ARCH" = "arm64" ]; then
        NODE_ARCH="darwin-arm64"
    else
        NODE_ARCH="darwin-x64"
    fi
    NODE_DIR="$HOME/.local/nodejs"

    echo "  Downloading Node.js $NODE_VER ($NODE_ARCH)..."
    TMPDIR_NODE=$(mktemp -d)
    curl -fsSL "https://nodejs.org/dist/$NODE_VER/node-$NODE_VER-$NODE_ARCH.tar.gz" -o "$TMPDIR_NODE/node.tar.gz"
    echo "  Extracting to $NODE_DIR..."
    mkdir -p "$NODE_DIR"
    tar -xzf "$TMPDIR_NODE/node.tar.gz" -C "$NODE_DIR" --strip-components=1
    rm -rf "$TMPDIR_NODE"

    # Add to PATH for this session
    export PATH="$NODE_DIR/bin:$PATH"

    echo "  Installed: $(node --version)"
    echo "  NOTE: To use node outside this script, add to your shell profile:"
    echo "    export PATH=\"$NODE_DIR/bin:\$PATH\""
fi

# ─── 4. Build ───────────────────────────────────────────────────────────────
echo ""
echo "[4/4] Building DJ USB Tkit..."

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

cd "$PROJECT_ROOT"

# Install frontend dependencies
echo "  Installing frontend dependencies..."
(cd vanilla-ui && npm ci)
(cd desktop && npm ci)

# Build frontend dist
echo "  Building frontend dist..."
(cd vanilla-ui && npm run build)

# Do not bundle Node runtime in release artifacts.
echo "  Ensuring no bundled Node runtime is staged..."
rm -rf "$PROJECT_ROOT/desktop/runtime/bin" "$PROJECT_ROOT/desktop/runtime/node_modules"

# Install Tauri CLI if needed
if ! command -v cargo-tauri &>/dev/null; then
    echo "  Installing Tauri CLI..."
    cargo install tauri-cli --version '^2' --locked
fi

echo "  Building (this will take a while on first run)..."
cd "$PROJECT_ROOT/desktop/src-tauri"
cargo tauri build --config "$PROJECT_ROOT/scripts/tauri.release.conf.json" --bundles app,dmg

# ─── Done ────────────────────────────────────────────────────────────────────
BUNDLE_DIR="$PROJECT_ROOT/desktop/src-tauri/target/release/bundle"
echo ""
echo "=== Build complete! ==="
echo "Installers are in: $BUNDLE_DIR"
echo "  - DMG: $BUNDLE_DIR/dmg"
echo "  - App: $BUNDLE_DIR/macos"
open "$BUNDLE_DIR"
