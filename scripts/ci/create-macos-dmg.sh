#!/usr/bin/env bash
# Build a compressed UDZO DMG with Spec Chum.app and an Applications shortcut.
#
# Usage:
#   create-macos-dmg.sh <version> <Spec-Chum.app> <output.dmg> [extra-file...]
#
# Example:
#   create-macos-dmg.sh 0.2.0 "dist/out/Spec Chum.app" packed/out.dmg \
#     dist/out/LICENSE dist/out/README.txt
#
# Notarisation / stapling is intentionally out of scope here (follow-up on #231).
set -euo pipefail

if [[ $# -lt 3 ]]; then
  echo "usage: $0 <version> <Spec-Chum.app> <output.dmg> [extra-file...]" >&2
  exit 2
fi

VERSION="$1"
APP_SRC="$2"
DMG_OUT="$3"
shift 3

if [[ ! -d "$APP_SRC" || ! -f "$APP_SRC/Contents/Info.plist" ]]; then
  echo "error: not a macOS .app bundle: $APP_SRC" >&2
  exit 1
fi

# Volume name: keep it short for Finder; avoid path separators.
VOL_NAME="Spec Chum ${VERSION}"
if [[ ${#VOL_NAME} -gt 27 ]]; then
  VOL_NAME="Spec Chum"
fi

TMP="${RUNNER_TEMP:-${TMPDIR:-/tmp}}/spec-chum-dmg-$$"
STAGE="$TMP/stage"
RW_DMG="$TMP/rw.dmg"
mkdir -p "$STAGE"
cleanup() {
  rm -rf "$TMP"
}
trap cleanup EXIT

# ditto preserves codesign attributes better than cp -R on macOS.
ditto "$APP_SRC" "$STAGE/Spec Chum.app"
ln -s /Applications "$STAGE/Applications"

for extra in "$@"; do
  if [[ ! -e "$extra" ]]; then
    echo "error: extra file not found: $extra" >&2
    exit 1
  fi
  cp "$extra" "$STAGE/"
done

mkdir -p "$(dirname "$DMG_OUT")"
rm -f "$DMG_OUT" "$RW_DMG"

# Create a writable image first so Finder can resolve the Applications symlink
# when users open the final compressed DMG; then convert to UDZO.
hdiutil create \
  -volname "$VOL_NAME" \
  -srcfolder "$STAGE" \
  -ov \
  -format UDRW \
  "$RW_DMG"
hdiutil convert "$RW_DMG" -format UDZO -imagekey zlib-level=9 -o "$DMG_OUT"
rm -f "$RW_DMG"

# Sanity: mounted volume should expose the .app and Applications link.
# Use a private mountroot so we do not depend on parsing tab-separated
# hdiutil paths that contain spaces (volume name "Spec Chum …").
MOUNT_ROOT="$TMP/mnt"
mkdir -p "$MOUNT_ROOT"
hdiutil attach -nobrowse -readonly -mountroot "$MOUNT_ROOT" "$DMG_OUT" >/dev/null
MOUNT_POINT="$(find "$MOUNT_ROOT" -mindepth 1 -maxdepth 1 -type d | head -n 1)"
if [[ -z "${MOUNT_POINT:-}" || ! -d "$MOUNT_POINT" ]]; then
  echo "error: failed to attach DMG for verification: $DMG_OUT" >&2
  exit 1
fi
detach_dmg() {
  hdiutil detach "$MOUNT_POINT" -quiet || hdiutil detach "$MOUNT_POINT" -force || true
}
trap 'detach_dmg; cleanup' EXIT

test -d "$MOUNT_POINT/Spec Chum.app/Contents/MacOS"
test -L "$MOUNT_POINT/Applications"
test "$(readlink "$MOUNT_POINT/Applications")" = "/Applications"

detach_dmg
trap cleanup EXIT

echo "created $DMG_OUT (volume \"$VOL_NAME\")"
