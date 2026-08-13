#!/usr/bin/env bash
# Build the native macOS Spec Chum shell (Rust host_api + SwiftUI).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

# SwiftUI macros require a full Xcode toolchain (not Command Line Tools alone).
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
if [[ -z "${DEVELOPER_DIR:-}" ]]; then
  echo "error: install Xcode (or set DEVELOPER_DIR). Command Line Tools alone cannot build SwiftUI." >&2
  exit 1
fi
echo "==> Using DEVELOPER_DIR=$DEVELOPER_DIR"

echo "==> cargo build -p host_api --release"
cargo build -p host_api --release

# Make the dylib relocatable via @rpath (cargo defaults to an absolute install name).
DYLIB="$ROOT/target/release/libspec_chum_host.dylib"
if [[ -f "$DYLIB" ]]; then
  install_name_tool -id '@rpath/libspec_chum_host.dylib' "$DYLIB" 2>/dev/null || true
  # Keep deps/ copy in sync when present (linker may resolve either).
  if [[ -f "$ROOT/target/release/deps/libspec_chum_host.dylib" ]]; then
    cp -f "$DYLIB" "$ROOT/target/release/deps/libspec_chum_host.dylib"
  fi
fi

# Keep the Swift package header in sync with the Rust crate.
mkdir -p apps/macos/Sources/CSpecChumHost/include
cp crates/host_api/include/spec_chum_host.h apps/macos/Sources/CSpecChumHost/include/spec_chum_host.h

echo "==> swift build (SpecChumMac)"
export SPEC_CHUM_ROOT="$ROOT"
# Prefer xcrun so the selected Xcode's swift is used.
xcrun swift build -c release --package-path apps/macos

BIN="$ROOT/apps/macos/.build/release/SpecChumMac"
if [[ -x "$BIN" ]]; then
  # Rewrite absolute cargo load path to @rpath if the linker baked one in.
  ABS_DEPS="$ROOT/target/release/deps/libspec_chum_host.dylib"
  ABS_REL="$ROOT/target/release/libspec_chum_host.dylib"
  install_name_tool -change "$ABS_DEPS" '@rpath/libspec_chum_host.dylib' "$BIN" 2>/dev/null || true
  install_name_tool -change "$ABS_REL" '@rpath/libspec_chum_host.dylib' "$BIN" 2>/dev/null || true
fi

echo ""
echo "Built: $BIN"
echo "Run with:"
echo "  ./scripts/run_macos_app.sh"
echo "  # (stages SpecChumMac.app and open(1) so SpecChum becomes key — do not type in Terminal)"
