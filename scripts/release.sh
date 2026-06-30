#!/usr/bin/env bash
set -euo pipefail

SCRIPT_PATH="$(realpath "${BASH_SOURCE[0]}")"
SCRIPT_DIR="$(dirname "$SCRIPT_PATH")"
ROOT_DIR="$(
  git -C "$SCRIPT_DIR" rev-parse --show-toplevel 2>/dev/null \
    || (cd "$SCRIPT_DIR/.." && pwd)
)"
BACKEND_DIR="$ROOT_DIR/backend"
UI_DIR="$ROOT_DIR/vanilla-ui"
TAURI_DIR="$ROOT_DIR/desktop/src-tauri"
FRONTEND_DIST_DIR="$UI_DIR/dist"
TAURI_RELEASE_CONFIG="$ROOT_DIR/scripts/tauri.release.conf.json"
FRONTEND_DIST_READY=0

RUN_TESTS="${RUN_TESTS:-1}"
TARGETS="${TARGETS:-}"
EXTRA_TAURI_ARGS="${EXTRA_TAURI_ARGS:-}"
AUTO_INSTALL_TAURI="${AUTO_INSTALL_TAURI:-1}"
BUNDLES="${BUNDLES:-}"
LINUXDEPLOY_NO_STRIP="${LINUXDEPLOY_NO_STRIP:-1}"
RUNTIME_BIN_DIR="$ROOT_DIR/desktop/runtime/bin"
RUNTIME_NODE_MODULES_DIR="$ROOT_DIR/desktop/runtime/node_modules"

echo "==> Host: $(uname -s)"
echo "==> Script: $SCRIPT_PATH"
echo "==> Project root: $ROOT_DIR"

if [[ ! -d "$BACKEND_DIR" || ! -d "$UI_DIR" || ! -d "$TAURI_DIR" ]]; then
  echo "error: expected project directories were not found under: $ROOT_DIR" >&2
  exit 1
fi

if [[ ! -f "$TAURI_RELEASE_CONFIG" ]]; then
  echo "error: missing release config: $TAURI_RELEASE_CONFIG" >&2
  exit 1
fi

if ! command -v cargo >/dev/null 2>&1; then
  echo "error: cargo is required" >&2
  exit 1
fi

if ! command -v npm >/dev/null 2>&1; then
  echo "error: npm is required" >&2
  exit 1
fi

export PATH="$HOME/.cargo/bin:$PATH"
TAURI_CMD=()
TAURI_BIN=""
resolve_tauri_cmd() {
  local tauri_cli_path=""

  if [[ -n "${CARGO_HOME:-}" && -x "$CARGO_HOME/bin/cargo-tauri" ]]; then
    tauri_cli_path="$CARGO_HOME/bin/cargo-tauri"
  elif [[ -x "$HOME/.cargo/bin/cargo-tauri" ]]; then
    tauri_cli_path="$HOME/.cargo/bin/cargo-tauri"
  elif [[ -x "$BACKEND_DIR/.cargo/bin/cargo-tauri" ]]; then
    tauri_cli_path="$BACKEND_DIR/.cargo/bin/cargo-tauri"
  elif [[ -x "$BACKEND_DIR/bin/cargo-tauri" ]]; then
    tauri_cli_path="$BACKEND_DIR/bin/cargo-tauri"
  fi

  if [[ -n "$tauri_cli_path" ]]; then
    TAURI_BIN="$tauri_cli_path"
    TAURI_CMD=(cargo tauri)
    return 0
  fi

  if cargo tauri --help >/dev/null 2>&1; then
    TAURI_BIN="$(command -v cargo)"
    TAURI_CMD=(cargo tauri)
    return 0
  fi

  if command -v cargo-tauri >/dev/null 2>&1; then
    tauri_cli_path="$(command -v cargo-tauri)"
    case "$tauri_cli_path" in
      /usr/bin/*|/usr/local/bin/*)
        echo "warning: ignoring cargo-tauri on PATH at $tauri_cli_path" >&2
        echo "         it may be an incompatible standalone binary for this runner." >&2
        ;;
      *)
        TAURI_BIN="$tauri_cli_path"
        TAURI_CMD=(cargo tauri)
        return 0
        ;;
    esac
  fi

  return 1
}

if ! resolve_tauri_cmd; then
  if [[ "$AUTO_INSTALL_TAURI" == "1" ]]; then
    echo "==> Tauri CLI not found, installing tauri-cli v2"
    cargo install tauri-cli --version '^2' --locked --force
    resolve_tauri_cmd || {
      echo "error: tauri-cli install completed but CLI still unavailable." >&2
      echo "       ensure ~/.cargo/bin is on PATH or rerun with explicit CARGO_HOME." >&2
      exit 1
    }
  else
    echo "error: tauri CLI is not installed or not on PATH." >&2
    echo "       looked in: $BACKEND_DIR/.cargo/bin, $BACKEND_DIR/bin, ~/.cargo/bin, \$CARGO_HOME/bin" >&2
    echo "       install with: cargo install tauri-cli --version '^2'" >&2
    echo "       and ensure ~/.cargo/bin is on PATH" >&2
    echo "       or run script with AUTO_INSTALL_TAURI=1" >&2
    exit 1
  fi
fi

echo "==> Tauri command: ${TAURI_CMD[*]}"
if [[ -n "$TAURI_BIN" ]]; then
  echo "==> Tauri CLI binary: $TAURI_BIN"
fi

if [[ -z "$BUNDLES" ]]; then
  case "$(uname -s)" in
    Linux)
      BUNDLES="deb,rpm,appimage"
      ;;
    Darwin)
      BUNDLES="app,dmg"
      ;;
    MINGW*|MSYS*|CYGWIN*|Windows_NT)
      BUNDLES="nsis,msi"
      ;;
    *)
      echo "error: unsupported host for default bundles: $(uname -s)" >&2
      echo "       set BUNDLES explicitly, e.g. BUNDLES=appimage" >&2
      exit 1
      ;;
  esac
fi
echo "==> Bundles: $BUNDLES"

if [[ "$(uname -s)" == "Linux" && "$BUNDLES" == *"appimage"* ]]; then
  if ! command -v linuxdeploy >/dev/null 2>&1; then
    echo "error: AppImage bundle requested but 'linuxdeploy' is not available on PATH." >&2
    echo "       install linuxdeploy or rerun without AppImage, e.g.:" >&2
    echo "       BUNDLES=deb,rpm ./scripts/release.sh" >&2
    exit 1
  fi
fi

if [[ "$(uname -s)" == "Linux" && "$BUNDLES" == *"appimage"* ]]; then
  if [[ "$LINUXDEPLOY_NO_STRIP" == "1" ]]; then
    # Arch and other modern distros may ship RELR-enabled system libraries that
    # the bundled linuxdeploy strip binary cannot process.
    export NO_STRIP=1
    echo "==> AppImage: NO_STRIP=1 (linuxdeploy strip workaround for RELR libs)"
  fi

  # GPU and display-server libraries are host-specific — they wrap the GPU
  # driver and must come from the running system, not the bundle. Bundling them
  # causes EGL_BAD_PARAMETER crashes on Wayland and GL dispatch mismatches on X11.
  export LINUXDEPLOY_EXCLUDE_LIBS="libwayland-egl.so.1:libEGL.so.1:libGL.so.1:libGLX.so.0:libGLdispatch.so.0:libvulkan.so.1"
  echo "==> AppImage: excluding GPU/display libs: $LINUXDEPLOY_EXCLUDE_LIBS"
fi

cd "$ROOT_DIR"

prepare_packaged_js_runtime() {
  # Node.js runtime is no longer bundled in release builds.
  # BPM/key analysis uses the built-in stratum-dsp engine by default.
  # Users who want essentia.js can point DJTKIT_ESSENTIA_NODE at their own
  # system Node and the essentia_runner.cjs + desktop/node_modules are still
  # available for that opt-in path.

  echo "==> Syncing package versions from root Cargo workspace"
  node "$ROOT_DIR/scripts/sync_versions.mjs" >/dev/null

  echo "==> Installing desktop JS dependencies (build-time only)"
  (
    cd "$ROOT_DIR/desktop"
    npm ci
  )

  # Clean any previously-bundled Node runtime artifacts
  rm -rf "$RUNTIME_BIN_DIR" "$RUNTIME_NODE_MODULES_DIR"
}

prepare_frontend_dist() {
  if [[ "$FRONTEND_DIST_READY" == "1" && -f "$FRONTEND_DIST_DIR/index.html" ]]; then
    echo "==> Reusing existing frontend dist at $FRONTEND_DIST_DIR"
    return
  fi

  echo "==> Preparing frontend dist at $FRONTEND_DIST_DIR"
  (
    cd "$UI_DIR"
    npm run build
  )

  if [[ ! -f "$FRONTEND_DIST_DIR/index.html" ]]; then
    echo "error: frontend dist missing index.html after staging" >&2
    exit 1
  fi
}

clean_tauri_bundle_output() {
  local target_triple="${1:-}"
  local bundle_dir
  if [[ -n "$target_triple" ]]; then
    bundle_dir="$ROOT_DIR/target/$target_triple/release/bundle"
  else
    bundle_dir="$ROOT_DIR/target/release/bundle"
  fi
  if [[ -d "$bundle_dir" ]]; then
    echo "==> Clearing stale Tauri bundle output at $bundle_dir"
    rm -rf "$bundle_dir"
  fi
}

if [[ "$RUN_TESTS" == "1" ]]; then
  echo "==> Running backend tests"
  (
    cd "$BACKEND_DIR"
    cargo test
  )

  echo "==> Running frontend tests"
  (
    cd "$UI_DIR"
    npm run test:ci
  )
  FRONTEND_DIST_READY=1
else
  echo "==> Skipping tests (RUN_TESTS=$RUN_TESTS)"
fi

prepare_frontend_dist

echo "==> Building Tauri bundles"
if [[ -n "$TARGETS" ]]; then
  IFS=',' read -r -a target_array <<< "$TARGETS"
  for target in "${target_array[@]}"; do
    target_trimmed="$(echo "$target" | xargs)"
    [[ -z "$target_trimmed" ]] && continue
    clean_tauri_bundle_output "$target_trimmed"
    prepare_packaged_js_runtime "$target_trimmed"
    echo "==> Building target: $target_trimmed"
    (
      cd "$TAURI_DIR"
      "${TAURI_CMD[@]}" build --config "$TAURI_RELEASE_CONFIG" --bundles "$BUNDLES" --target "$target_trimmed" $EXTRA_TAURI_ARGS
    )
    echo "==> Done"
    echo "Artifacts:"
    echo "  $ROOT_DIR/target/$target_trimmed/release/bundle"
    echo
  done
else
  clean_tauri_bundle_output
  prepare_packaged_js_runtime
  (
    cd "$TAURI_DIR"
    "${TAURI_CMD[@]}" build --config "$TAURI_RELEASE_CONFIG" --bundles "$BUNDLES" $EXTRA_TAURI_ARGS
  )
  echo "==> Done"
  echo "Artifacts:"
  echo "  $ROOT_DIR/target/release/bundle"
  echo
fi

echo "Note: Cross-platform release requires running this on each OS (or CI matrix)."
