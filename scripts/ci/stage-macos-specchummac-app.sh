#!/usr/bin/env bash
# Stage SpecChumMac into a production macOS .app for release DMGs (#363).
#
# Usage:
#   stage-macos-specchummac-app.sh <version> <SpecChumMac-binary> <dest-app-path> [repo-root]
#
# Optional 4th arg is the checkout used for living_room assets + bundled ROMs
# (defaults to the repo containing this script). Release CI fetches ROMs first.
#
# The CFBundleExecutable is the Mach-O binary (not a shell wrapper) so
# codesign / notary hardened-runtime paths stay valid (#354).
set -euo pipefail

if [[ $# -lt 3 || $# -gt 4 ]]; then
  echo "usage: $0 <version> <SpecChumMac-binary> <dest-app-path> [repo-root]" >&2
  exit 2
fi

VERSION="$1"
BIN_SRC="$2"
APP_DST="$3"
SCRIPT_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
REPO_ROOT="$(cd "${4:-$SCRIPT_ROOT}" && pwd)"

if [[ ! -f "$BIN_SRC" ]]; then
  echo "error: binary not found: $BIN_SRC" >&2
  exit 1
fi
if [[ ! -x "$BIN_SRC" ]]; then
  chmod +x "$BIN_SRC"
fi

rm -rf "$APP_DST"
MACOS_DIR="$APP_DST/Contents/MacOS"
RESOURCES_DIR="$APP_DST/Contents/Resources"
mkdir -p "$MACOS_DIR" "$RESOURCES_DIR"

cp "$BIN_SRC" "$MACOS_DIR/SpecChumMac"
chmod +x "$MACOS_DIR/SpecChumMac"
strip "$MACOS_DIR/SpecChumMac" 2>/dev/null || true

ICNS="$SCRIPT_ROOT/packaging/macos/AppIcon.icns"
if [[ ! -f "$ICNS" ]]; then
  # Tag checkout may lack packaging/; fall back to release-ref repo.
  ICNS="$REPO_ROOT/packaging/macos/AppIcon.icns"
fi
if [[ ! -f "$ICNS" ]]; then
  echo "error: shared app icon missing (packaging/macos/AppIcon.icns)" >&2
  exit 1
fi
cp "$ICNS" "$RESOURCES_DIR/AppIcon.icns"

# Living-room Bevy assets (optional if fetch skipped; warn only).
if [[ -x "$REPO_ROOT/scripts/stage_living_room_assets.sh" ]]; then
  "$REPO_ROOT/scripts/stage_living_room_assets.sh" \
    "$REPO_ROOT" "$RESOURCES_DIR/living_room_assets"
fi

# Redistributable ROMs into Contents/Resources/roms (#363 addendum).
BUNDLE_ROMS="$SCRIPT_ROOT/scripts/ci/bundle-release-roms.sh"
if [[ ! -x "$BUNDLE_ROMS" ]]; then
  chmod +x "$BUNDLE_ROMS" 2>/dev/null || true
fi
if [[ ! -f "$BUNDLE_ROMS" ]]; then
  echo "error: missing bundle-release-roms.sh beside trusted CI tree" >&2
  exit 1
fi
"$BUNDLE_ROMS" "$REPO_ROOT" "$RESOURCES_DIR"
# NOTICE lives next to roms/ inside Resources for Finder visibility.
if [[ -f "$RESOURCES_DIR/ROMS-NOTICE.txt" ]]; then
  :
fi

plist_escape() {
  printf '%s' "$1" | sed -e 's/&/\&amp;/g' -e 's/</\&lt;/g' -e 's/>/\&gt;/g'
}
BUNDLE_VERSION="$VERSION"
if [[ ! "$BUNDLE_VERSION" =~ ^[0-9]+(\.[0-9]+){0,2}$ ]]; then
  BUNDLE_VERSION="0.0.0"
fi
VERSION_XML="$(plist_escape "$BUNDLE_VERSION")"

cat > "$APP_DST/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleDevelopmentRegion</key>
	<string>en</string>
	<key>CFBundleDisplayName</key>
	<string>Spec Chum</string>
	<key>CFBundleExecutable</key>
	<string>SpecChumMac</string>
	<key>CFBundleIconFile</key>
	<string>AppIcon</string>
	<key>CFBundleIdentifier</key>
	<string>dev.specchum.SpecChumMac</string>
	<key>CFBundleInfoDictionaryVersion</key>
	<string>6.0</string>
	<key>CFBundleName</key>
	<string>Spec Chum</string>
	<key>CFBundlePackageType</key>
	<string>APPL</string>
	<key>CFBundleShortVersionString</key>
	<string>${VERSION_XML}</string>
	<key>CFBundleVersion</key>
	<string>${VERSION_XML}</string>
	<key>LSMinimumSystemVersion</key>
	<string>14.0</string>
	<key>NSHighResolutionCapable</key>
	<true/>
	<key>NSWindowTabbingEnabled</key>
	<false/>
	<key>NSPrincipalClass</key>
	<string>NSApplication</string>
	<key>NSBluetoothAlwaysUsageDescription</key>
	<string>Spec Chum uses Bluetooth to discover wireless game controllers.</string>
</dict>
</plist>
PLIST

printf 'APPL????' > "$APP_DST/Contents/PkgInfo"

echo "staged $APP_DST (SpecChumMac, version ${VERSION})"
