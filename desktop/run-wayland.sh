#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TAURI_DIR="${SCRIPT_DIR}/src-tauri"
UI_DIR="${SCRIPT_DIR}/../vanilla-ui"

export GDK_BACKEND=wayland
export WINIT_UNIX_BACKEND=wayland
export WEBKIT_DISABLE_DMABUF_RENDERER=1

node "${SCRIPT_DIR}/../scripts/sync_versions.mjs" >/dev/null

cd "${UI_DIR}"
npm run build

cd "${TAURI_DIR}"
exec cargo run "$@"
