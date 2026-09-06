#!/usr/bin/env bash
# Build the native macOS Spec Chum shell (Rust host_api via living_room staticlib + SwiftUI).
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

# Match Package.swift `.macOS(.v14)` / Info.plist LSMinimumSystemVersion.
# Without this, clang-built deps (notably blake3 NEON asm) stamp the host SDK
# (e.g. 26.x/27.x) and Swift link emits "built for newer macOS than being linked (14.0)".
# Refs #171.
export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-14.0}"
export CFLAGS="${CFLAGS:--mmacosx-version-min=${MACOSX_DEPLOYMENT_TARGET}}"
export CXXFLAGS="${CXXFLAGS:--mmacosx-version-min=${MACOSX_DEPLOYMENT_TARGET}}"
echo "==> MACOSX_DEPLOYMENT_TARGET=$MACOSX_DEPLOYMENT_TARGET (Rust + C deps)"

echo "==> cargo build -p living_room --release --no-default-features (embed staticlib + host_api)"
# strip=none is set in workspace profile for living_room (macOS 27 LINKEDIT).
# --no-default-features omits standalone Bevy chrome / cpal / rfd (Swift owns those).
cargo build -p living_room --release --no-default-features

if [[ ! -f "$ROOT/target/release/libspec_chum_room.a" ]]; then
  echo "error: missing target/release/libspec_chum_room.a" >&2
  exit 1
fi

# Keep the Swift package headers in sync with the Rust crates.
# `spec_chum_room.h` is a symlink to the crate SoT (avoid a second hand-maintained copy).
mkdir -p apps/macos/Sources/CSpecChumHost/include
cp crates/host_api/include/spec_chum_host.h apps/macos/Sources/CSpecChumHost/include/spec_chum_host.h
ROOM_H="apps/macos/Sources/CSpecChumHost/include/spec_chum_room.h"
ROOM_SRC="../../../../../crates/living_room/include/spec_chum_room.h"
if [[ -L "$ROOM_H" ]]; then
  :
elif [[ -e "$ROOM_H" ]]; then
  rm -f "$ROOM_H"
  ln -s "$ROOM_SRC" "$ROOM_H"
else
  ln -s "$ROOM_SRC" "$ROOM_H"
fi

echo "==> swift build (SpecChumMac, force_load libspec_chum_room.a)"
export SPEC_CHUM_ROOT="$ROOT"
xcrun swift build -c release --package-path apps/macos

BIN="$ROOT/apps/macos/.build/release/SpecChumMac"
APP_STAGE="$ROOT/apps/macos/.build/SpecChumMac.app"
RESOURCES="$APP_STAGE/Contents/Resources"
mkdir -p "$RESOURCES"
echo "==> copy living_room assets → SpecChumMac.app Resources"
"$ROOT/scripts/stage_living_room_assets.sh" "$ROOT" "$RESOURCES/living_room_assets"

echo ""
echo "Built: $BIN"
echo "Run with:"
echo "  ./scripts/run_macos_app.sh"
