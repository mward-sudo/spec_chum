#!/usr/bin/env bash
# Build (if needed) and run the native macOS Spec Chum app from the repo root.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="$ROOT/apps/macos/.build/release/SpecChumMac"
DYLIB="$ROOT/target/release/libspec_chum_host.dylib"

if [[ -z "${DEVELOPER_DIR:-}" ]]; then
  for candidate in \
    /Applications/Xcode.app/Contents/Developer \
    /Applications/Xcode-beta.app/Contents/Developer; do
    if [[ -d "$candidate" ]]; then
      export DEVELOPER_DIR="$candidate"
      break
    fi
  done
fi

if [[ ! -x "$BIN" || ! -f "$DYLIB" ]]; then
  "$ROOT/scripts/build_macos_app.sh"
fi

export SPEC_CHUM_ROOT="$ROOT"
export DYLD_LIBRARY_PATH="$ROOT/target/release${DYLD_LIBRARY_PATH:+:$DYLD_LIBRARY_PATH}"
cd "$ROOT"
exec "$BIN"
