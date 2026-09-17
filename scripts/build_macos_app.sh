#!/usr/bin/env bash
# Build the native macOS Spec Chum shell (Rust living_room staticlib + SwiftUI).
#
# Optional env:
#   SPEC_CHUM_MAC_TARGET  — Rust target triple (e.g. x86_64-apple-darwin).
#                           When set and not host-native, builds with
#                           `cargo --target` and `swift build --arch`.
#   CARGO_TARGET_DIR       — Cargo artifact dir (see scripts/dev_env.sh / External SSD).
#   SPEC_CHUM_HOST_LIB_DIR — override linker search dir for libspec_chum_room.a
#                           (default: $CARGO_TARGET_DIR/release or …/<triple>/release).
#   SPEC_CHUM_SWIFT_SCRATCH — SwiftPM --scratch-path (default: apps/macos/.build).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

# Prefer already-set cache env; otherwise load SSD/local defaults from dev_env.sh.
if [[ -z "${CARGO_TARGET_DIR:-}${SPEC_CHUM_HOST_LIB_DIR:-}${SPEC_CHUM_SWIFT_SCRATCH:-}" ]]; then
  if [[ -f "$ROOT/scripts/dev_env.sh" ]]; then
    # shellcheck disable=SC1091
    source "$ROOT/scripts/dev_env.sh"
  fi
fi

CARGO_DIR="${CARGO_TARGET_DIR:-$ROOT/target}"
SWIFT_SCRATCH="${SPEC_CHUM_SWIFT_SCRATCH:-$ROOT/apps/macos/.build}"

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
# Append so a pre-set CFLAGS/CXXFLAGS cannot drop the deployment min (CR on #312).
export CFLAGS="${CFLAGS:+$CFLAGS }-mmacosx-version-min=${MACOSX_DEPLOYMENT_TARGET}"
export CXXFLAGS="${CXXFLAGS:+$CXXFLAGS }-mmacosx-version-min=${MACOSX_DEPLOYMENT_TARGET}"
echo "==> MACOSX_DEPLOYMENT_TARGET=$MACOSX_DEPLOYMENT_TARGET (Rust + C deps)"

HOST_ARCH="$(uname -m)"
TARGET_TRIPLE="${SPEC_CHUM_MAC_TARGET:-}"
CARGO_TARGET_ARGS=()
SWIFT_ARCH_ARGS=()
LIB_DIR="$CARGO_DIR/release"

if [[ -n "$TARGET_TRIPLE" ]]; then
  echo "==> SPEC_CHUM_MAC_TARGET=$TARGET_TRIPLE"
  CARGO_TARGET_ARGS=(--target "$TARGET_TRIPLE")
  LIB_DIR="$CARGO_DIR/${TARGET_TRIPLE}/release"
  case "$TARGET_TRIPLE" in
    aarch64-apple-darwin)
      SWIFT_ARCH_ARGS=(--arch arm64)
      ;;
    x86_64-apple-darwin)
      SWIFT_ARCH_ARGS=(--arch x86_64)
      ;;
    *)
      echo "error: unsupported SPEC_CHUM_MAC_TARGET=$TARGET_TRIPLE" >&2
      exit 1
      ;;
  esac
  # When targeting the host arch, cargo still accepts --target; Swift --arch
  # matches. Cross-arch (e.g. x86_64 on arm64 runner) needs both.
  if [[ "$TARGET_TRIPLE" == "aarch64-apple-darwin" && "$HOST_ARCH" == "arm64" ]] ||
     [[ "$TARGET_TRIPLE" == "x86_64-apple-darwin" && "$HOST_ARCH" == "x86_64" ]]; then
    # Native triple: prefer default target/release layout when no explicit target
    # was required — still use --target for a deterministic lib path in CI.
    :
  fi
fi

# Cross-target builds must use the triple-specific release dir even if
# scripts/dev_env.sh exported the native …/release default.
if [[ -n "$TARGET_TRIPLE" ]]; then
  export SPEC_CHUM_HOST_LIB_DIR="$LIB_DIR"
else
  export SPEC_CHUM_HOST_LIB_DIR="${SPEC_CHUM_HOST_LIB_DIR:-$LIB_DIR}"
fi

echo "==> cargo build -p living_room --release --no-default-features ${CARGO_TARGET_ARGS[*]:-}"
# strip=none is set in workspace profile for living_room (macOS 27 LINKEDIT).
# --no-default-features omits standalone Bevy chrome / cpal / rfd (Swift owns those).
if ((${#CARGO_TARGET_ARGS[@]})); then
  cargo build -p living_room --release --no-default-features "${CARGO_TARGET_ARGS[@]}"
else
  cargo build -p living_room --release --no-default-features
fi

ROOM_A="$SPEC_CHUM_HOST_LIB_DIR/libspec_chum_room.a"
if [[ ! -f "$ROOM_A" ]]; then
  echo "error: missing $ROOM_A" >&2
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

echo "==> swift build (SpecChumMac, force_load libspec_chum_room.a from $SPEC_CHUM_HOST_LIB_DIR)"
echo "==> Swift scratch: $SWIFT_SCRATCH"
export SPEC_CHUM_ROOT="$ROOT"
mkdir -p "$SWIFT_SCRATCH"
if ((${#SWIFT_ARCH_ARGS[@]})); then
  xcrun swift build -c release --package-path apps/macos --scratch-path "$SWIFT_SCRATCH" \
    "${SWIFT_ARCH_ARGS[@]}"
else
  xcrun swift build -c release --package-path apps/macos --scratch-path "$SWIFT_SCRATCH"
fi

BIN="$SWIFT_SCRATCH/release/SpecChumMac"
# Cross-arch SwiftPM may place the product under <scratch>/<arch>-apple-macosx/release/.
if [[ ! -x "$BIN" ]]; then
  case "${TARGET_TRIPLE:-}" in
    aarch64-apple-darwin) build_arch="arm64" ;;
    x86_64-apple-darwin) build_arch="x86_64" ;;
    *) build_arch="$HOST_ARCH" ;;
  esac
  BIN="$(find "$SWIFT_SCRATCH" -type f \
    -path "*/${build_arch}-apple-macosx/release/SpecChumMac" -print -quit || true)"
fi
if [[ -z "${BIN:-}" || ! -x "$BIN" ]]; then
  # Last resort: unique release product (native single-arch builds).
  BIN="$(find "$SWIFT_SCRATCH" -type f -name SpecChumMac -path '*/release/SpecChumMac' | head -n 1 || true)"
fi
if [[ -z "${BIN:-}" || ! -x "$BIN" ]]; then
  echo "error: SpecChumMac binary not found under $SWIFT_SCRATCH" >&2
  exit 1
fi

# force_load of libspec_chum_room.a makes ld stamp LC_BUILD_VERSION sdk from the
# Rust/C objects (14.0 via MACOSX_DEPLOYMENT_TARGET above). macOS then withholds
# Liquid Glass toolbar shared-background pills. Re-stamp minos=deployment,
# sdk=active Xcode SDK so SpecChumMac matches release chrome (minos 14 / sdk 26+).
if ! SDK_VERSION="$(xcrun --show-sdk-version 2>/dev/null)"; then
  echo "error: cannot determine the active macOS SDK version" >&2
  exit 1
fi
SDK_VERSION="${SDK_VERSION%%_*}" # drop rare suffixes
if [[ -z "$SDK_VERSION" ]]; then
  echo "error: active macOS SDK version is empty" >&2
  exit 1
fi
echo "==> vtool LC_BUILD_VERSION macos ${MACOSX_DEPLOYMENT_TARGET} / sdk ${SDK_VERSION}"
xcrun vtool -set-build-version macos "$MACOSX_DEPLOYMENT_TARGET" "$SDK_VERSION" \
  -replace -output "$BIN" "$BIN"

APP_STAGE="$SWIFT_SCRATCH/SpecChumMac.app"
RESOURCES="$APP_STAGE/Contents/Resources"
mkdir -p "$RESOURCES"
echo "==> ensure + copy living_room assets → SpecChumMac.app Resources"
"$ROOT/scripts/ensure_living_room_assets.sh" "$ROOT"
"$ROOT/scripts/stage_living_room_assets.sh" "$ROOT" "$RESOURCES/living_room_assets"

echo ""
echo "Built: $BIN"
echo "Run with:"
echo "  ./scripts/run_macos_app.sh"
# Export path for release CI consumers.
echo "$BIN" >"$SWIFT_SCRATCH/specchummac-binary-path.txt"
