#!/usr/bin/env bash
# Build Linux release inside a Docker container (Debian Bookworm, glibc 2.36)
# to ensure broad compatibility across distros.
#
# This script is intentionally isolated from scripts/release.sh so the Docker
# build path can diverge safely from the local host release flow.
#
# Usage:
#   ./scripts/linux-release-docker.sh
#   BUNDLES=appimage ./scripts/linux-release-docker.sh
#   RUN_TESTS=1 ./scripts/linux-release-docker.sh
#   REBUILD_IMAGE=1 ./scripts/linux-release-docker.sh   # force image rebuild

set -euo pipefail

SCRIPT_PATH="$(realpath "${BASH_SOURCE[0]}")"
SCRIPT_DIR="$(dirname "$SCRIPT_PATH")"
ROOT_DIR="$(
  git -C "$SCRIPT_DIR" rev-parse --show-toplevel 2>/dev/null \
    || (cd "$SCRIPT_DIR/.." && pwd)
)"

BUNDLES="${BUNDLES:-deb,rpm,appimage}"
RUN_TESTS="${RUN_TESTS:-0}"
IMAGE_NAME="${IMAGE_NAME:-djtkit-linux-build}"
REBUILD_IMAGE="${REBUILD_IMAGE:-}"
HOST_LOG_DIR="${HOST_LOG_DIR:-$ROOT_DIR/.docker-logs}"

echo "=== Building Linux release in Docker ==="
echo "  Project: $ROOT_DIR"
echo "  Bundles: $BUNDLES"
echo "  Run tests: $RUN_TESTS"
echo "  Host logs: $HOST_LOG_DIR"

if [[ ! -f "$SCRIPT_DIR/Dockerfile.linux-build" ]]; then
  echo "error: missing Docker build file: $SCRIPT_DIR/Dockerfile.linux-build" >&2
  exit 1
fi

if [[ ! -d "$ROOT_DIR/vanilla-ui" || ! -d "$ROOT_DIR/desktop/src-tauri" ]]; then
  echo "error: expected project directories were not found under: $ROOT_DIR" >&2
  exit 1
fi

if [[ ! -f "$ROOT_DIR/scripts/tauri.release.conf.json" ]]; then
  echo "error: missing Tauri release config: $ROOT_DIR/scripts/tauri.release.conf.json" >&2
  exit 1
fi

if [ -n "$REBUILD_IMAGE" ] || ! docker image inspect "$IMAGE_NAME" >/dev/null 2>&1; then
  echo "==> Building Docker image $IMAGE_NAME..."
  docker build -f "$SCRIPT_DIR/Dockerfile.linux-build" -t "$IMAGE_NAME" "$ROOT_DIR"
else
  echo "==> Using cached Docker image $IMAGE_NAME"
  echo "    (set REBUILD_IMAGE=1 to force rebuild)"
fi

mkdir -p "$HOST_LOG_DIR"
rm -f "$HOST_LOG_DIR"/container-build.log "$HOST_LOG_DIR"/tauri-appdir-inspection.log

HOST_UID="$(id -u)"
HOST_GID="$(id -g)"

docker run --rm \
  -v "$ROOT_DIR:/project" \
  -v "$HOST_LOG_DIR:/host-logs" \
  --privileged \
  -w /project \
  -e BUNDLES="$BUNDLES" \
  -e RUN_TESTS="$RUN_TESTS" \
  -e HOST_UID="$HOST_UID" \
  -e HOST_GID="$HOST_GID" \
  -e NO_STRIP="${NO_STRIP:-1}" \
  "$IMAGE_NAME" \
  bash -lc '
    set -euo pipefail

    export CARGO_HOME="${CARGO_HOME:-/root/.cargo}"
    export RUSTUP_HOME="${RUSTUP_HOME:-/root/.rustup}"
    export PATH="$CARGO_HOME/bin:/usr/local/cargo/bin:$PATH"

    UI_DIR="/project/vanilla-ui"
    DESKTOP_DIR="/project/desktop"
    TAURI_DIR="/project/desktop/src-tauri"
    WORKSPACE_TARGET_DIR="/project/target"
    DIST_DIR="/project/vanilla-ui/dist"
    TAURI_RELEASE_CONFIG="/project/scripts/tauri.release.conf.json"
    HOST_LOG_DIR="/host-logs"

    if [[ ! -d "$UI_DIR" || ! -d "$DESKTOP_DIR" || ! -d "$TAURI_DIR" ]]; then
      echo "error: expected project directories are missing inside container" >&2
      exit 1
    fi

    if [[ ! -f "$TAURI_RELEASE_CONFIG" ]]; then
      echo "error: missing Tauri release config inside container: $TAURI_RELEASE_CONFIG" >&2
      exit 1
    fi

    if ! command -v cargo >/dev/null 2>&1; then
      echo "==> cargo not on PATH, probing standard install locations"
      for cargo_candidate in \
        "$CARGO_HOME/bin/cargo" \
        /usr/local/cargo/bin/cargo \
        /root/.cargo/bin/cargo
      do
        if [[ -x "$cargo_candidate" ]]; then
          export PATH="$(dirname "$cargo_candidate"):$PATH"
          break
        fi
      done
    fi

    if ! command -v cargo >/dev/null 2>&1; then
      echo "error: cargo is required inside the container" >&2
      echo "       checked PATH=$PATH" >&2
      exit 1
    fi

    if ! command -v npm >/dev/null 2>&1; then
      echo "error: npm is required inside the container" >&2
      exit 1
    fi

    if ! command -v cargo-tauri >/dev/null 2>&1 && ! cargo tauri --help >/dev/null 2>&1; then
      echo "error: Tauri CLI is not available inside the container" >&2
      exit 1
    fi

    if [[ "$RUN_TESTS" == "1" ]]; then
      echo "==> Running backend tests"
      (
        cd /project/backend
        cargo test
      )
    else
      echo "==> Skipping backend tests (RUN_TESTS=$RUN_TESTS)"
    fi

    echo "==> Installing frontend dependencies"
    (
      cd "$UI_DIR"
      npm ci
    )
    (
      cd "$DESKTOP_DIR"
      npm ci
    )

    echo "==> Building frontend dist"
    (
      cd "$UI_DIR"
      npm run build
    )

    if [[ ! -f "$DIST_DIR/index.html" ]]; then
      echo "error: frontend dist missing index.html at $DIST_DIR/index.html" >&2
      exit 1
    fi

    # Release policy: do not bundle a Node runtime. Essentia support remains
    # optional via the bundled runner script and user-provided system Node.
    rm -rf /project/desktop/runtime/bin /project/desktop/runtime/node_modules

    TAURI_BUILD_CONFIG="$TAURI_RELEASE_CONFIG"

    if [[ ! -f "/project/desktop/scripts/essentia_runner.cjs" ]]; then
      echo "error: missing packaged analysis runner: /project/desktop/scripts/essentia_runner.cjs" >&2
      exit 1
    fi

    echo "==> Clearing stale bundle output"
    rm -rf "$WORKSPACE_TARGET_DIR/release/bundle" "$TAURI_DIR/target/release/bundle"

    echo "==> Verifying staging inputs"
    echo "    frontendDist: $DIST_DIR"
    echo "    runner:       /project/desktop/scripts/essentia_runner.cjs"
    ls -ld "$DIST_DIR" /project/desktop/scripts >/dev/null

    echo "==> AppImage/Tauri cache contents"
    if [[ -d /root/.cache/tauri ]]; then
      find /root/.cache/tauri -maxdepth 2 \( -type f -o -type l \) | sort
      if [[ -f /root/.cache/tauri/.cache-manifest ]]; then
        echo "==> Cached manifest snapshot"
        cat /root/.cache/tauri/.cache-manifest
      fi
    else
      echo "WARNING: /root/.cache/tauri does not exist" >&2
    fi

    if [[ ",$BUNDLES," == *",appimage,"* ]]; then
      # GPU and display-server libraries are host-specific — they wrap the GPU
      # driver and must come from the running system, not the bundle. Bundling them
      # causes EGL_BAD_PARAMETER crashes on Wayland and GL dispatch mismatches on X11.
      export LINUXDEPLOY_EXCLUDE_LIBS="libwayland-egl.so.1:libEGL.so.1:libGL.so.1:libGLX.so.0:libGLdispatch.so.0:libvulkan.so.1"
      echo "==> AppImage: excluding GPU/display libs: $LINUXDEPLOY_EXCLUDE_LIBS"

      echo "==> Verifying AppImage toolchain"
      for required_bin in \
        linuxdeploy linuxdeploy-plugin-appimage linuxdeploy-plugin-gtk \
        linuxdeploy-plugin-gstreamer AppRun zsyncmake desktop-file-validate appstreamcli
      do
        if ! command -v "$required_bin" >/dev/null 2>&1; then
          echo "error: required AppImage dependency is missing from PATH: $required_bin" >&2
          exit 1
        fi
        echo "    $required_bin -> $(command -v "$required_bin")"
      done

      echo "==> Verifying cached AppImage binaries for Tauri"
      for cached in \
        /root/.cache/tauri/linuxdeploy-x86_64.AppImage \
        /root/.cache/tauri/linuxdeploy-plugin-appimage-x86_64.AppImage
      do
        if [[ ! -f "$cached" || ! -x "$cached" ]]; then
          echo "error: cached AppImage binary missing or not executable: $cached" >&2
          exit 1
        fi
        echo "    ok: $cached ($(stat -c%s "$cached") bytes)"
      done
    fi

    echo "==> Building Tauri bundles: $BUNDLES"
    (
      cd "$TAURI_DIR"
      set +e
      if command -v cargo-tauri >/dev/null 2>&1; then
        cargo-tauri build --config "$TAURI_BUILD_CONFIG" --bundles "$BUNDLES" -v 2>&1 | tee "$HOST_LOG_DIR/container-build.log"
        build_status=${PIPESTATUS[0]}
      else
        cargo tauri build --config "$TAURI_BUILD_CONFIG" --bundles "$BUNDLES" -v 2>&1 | tee "$HOST_LOG_DIR/container-build.log"
        build_status=${PIPESTATUS[0]}
      fi
      set -e
      if [[ "$build_status" -ne 0 ]]; then
        if [[ ",$BUNDLES," == *",appimage,"* ]]; then
          REAL_APPDIR="$WORKSPACE_TARGET_DIR/release/bundle/appimage/DJ_USB_Tkit.AppDir"
          echo "==> Tauri AppDir inspection"
          if [[ -d "$REAL_APPDIR" ]]; then
            find "$REAL_APPDIR" | sort | tee "$HOST_LOG_DIR/tauri-appdir-inspection.log"
          else
            echo "Tauri AppDir missing: $REAL_APPDIR" | tee "$HOST_LOG_DIR/tauri-appdir-inspection.log"
          fi
        fi
        echo "==> Host logs directory: $HOST_LOG_DIR"
        exit "$build_status"
      fi
    )

    echo ""
    echo "=== Container build complete ==="
    echo "Bundles in: $WORKSPACE_TARGET_DIR/release/bundle/"

    chown -R "$HOST_UID:$HOST_GID" \
      /project/target \
      /project/desktop/src-tauri/target \
      /project/vanilla-ui/dist \
      2>/dev/null || true
  '

echo ""
echo "=== Done — output is in target/release/bundle/ ==="
echo "=== Logs are in $HOST_LOG_DIR ==="
