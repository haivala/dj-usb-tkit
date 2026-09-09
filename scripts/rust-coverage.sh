#!/usr/bin/env bash
set -euo pipefail

SCRIPT_PATH="$(realpath "${BASH_SOURCE[0]}")"
SCRIPT_DIR="$(dirname "$SCRIPT_PATH")"
ROOT_DIR="$(
  git -C "$SCRIPT_DIR" rev-parse --show-toplevel 2>/dev/null \
    || (cd "$SCRIPT_DIR/.." && pwd)
)"
OUT_DIR="$ROOT_DIR/target/llvm-cov"

MODE="text"

usage() {
  cat <<'USAGE'
Usage: scripts/rust-coverage.sh [--summary|--html|--lcov]

Runs local-only Rust coverage for the backend package with cargo-llvm-cov.

Modes:
  --summary  Print cargo-llvm-cov's compact terminal summary.
  --html     Write HTML coverage to target/llvm-cov/html.
  --lcov     Write LCOV coverage to target/llvm-cov/backend.lcov.

Default:
  Write a detailed text report to target/llvm-cov/backend.txt.
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --summary)
      MODE="summary"
      ;;
    --html)
      MODE="html"
      ;;
    --lcov)
      MODE="lcov"
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "error: unknown argument: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
  shift
done

if ! command -v cargo >/dev/null 2>&1; then
  echo "error: cargo is required" >&2
  exit 1
fi

if ! cargo llvm-cov --version >/dev/null 2>&1; then
  echo "error: cargo-llvm-cov is required" >&2
  echo "       install it with: cargo install cargo-llvm-cov" >&2
  exit 1
fi

if [[ -z "${LLVM_COV:-}" || -z "${LLVM_PROFDATA:-}" ]]; then
  if command -v llvm-cov >/dev/null 2>&1 && command -v llvm-profdata >/dev/null 2>&1; then
    export LLVM_COV="${LLVM_COV:-$(command -v llvm-cov)}"
    export LLVM_PROFDATA="${LLVM_PROFDATA:-$(command -v llvm-profdata)}"
  elif command -v rustup >/dev/null 2>&1 \
    && rustup component list --installed | grep -q '^llvm-tools-preview'; then
    :
  else
    echo "error: cargo-llvm-cov could not find LLVM coverage tools" >&2
    echo "       install the Rust llvm-tools-preview component, or set both" >&2
    echo "       LLVM_COV and LLVM_PROFDATA to matching system LLVM binaries." >&2
    exit 1
  fi
fi

mkdir -p "$OUT_DIR"
cd "$ROOT_DIR"

BASE_ARGS=(
  -p backend
  --lib
  --tests
  --ignore-filename-regex '(^|/)src/bin/|(^|/)src/tauri_commands\.rs$|(^|/)usr/src/debug/rust/'
)

case "$MODE" in
  summary)
    cargo llvm-cov "${BASE_ARGS[@]}"
    ;;
  html)
    cargo llvm-cov "${BASE_ARGS[@]}" --html --output-dir "$OUT_DIR/html"
    echo "HTML coverage written to $OUT_DIR/html/index.html"
    ;;
  lcov)
    cargo llvm-cov "${BASE_ARGS[@]}" --lcov --output-path "$OUT_DIR/backend.lcov"
    echo "LCOV coverage written to $OUT_DIR/backend.lcov"
    ;;
  text)
    cargo llvm-cov "${BASE_ARGS[@]}" --text --output-path "$OUT_DIR/backend.txt"
    echo "Text coverage written to $OUT_DIR/backend.txt"
    ;;
esac
