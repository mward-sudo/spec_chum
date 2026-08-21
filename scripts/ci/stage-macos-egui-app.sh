#!/usr/bin/env bash
# Stage the egui `spec_chum` binary into a production macOS .app bundle.
#
# SwiftUI SpecChumMac.app / DMG / notarisation remain [#68]; release CI ships this
# egui-wrapped bundle until that path is ready.
#
# Usage:
#   stage-macos-egui-app.sh <version> <spec_chum-binary> <dest-app-path>
#
# Example:
#   stage-macos-egui-app.sh 0.2.0 target/release/spec_chum "dist/out/Spec Chum.app"
set -euo pipefail

if [[ $# -ne 3 ]]; then
  echo "usage: $0 <version> <spec_chum-binary> <dest-app-path>" >&2
  exit 2
fi

VERSION="$1"
BIN_SRC="$2"
APP_DST="$3"

if [[ ! -f "$BIN_SRC" ]]; then
  echo "error: binary not found: $BIN_SRC" >&2
  exit 1
fi
if [[ ! -x "$BIN_SRC" ]]; then
  chmod +x "$BIN_SRC"
fi

# Replace any previous staging of this path.
rm -rf "$APP_DST"
MACOS_DIR="$APP_DST/Contents/MacOS"
mkdir -p "$MACOS_DIR"

# Keep the executable name space-free; display name comes from Info.plist.
cp "$BIN_SRC" "$MACOS_DIR/spec_chum"
chmod +x "$MACOS_DIR/spec_chum"
# Strip when possible (no-op on already-stripped or non-Mach-O in local dry runs).
strip "$MACOS_DIR/spec_chum" 2>/dev/null || true

# Escape XML special chars in version for plist text nodes.
plist_escape() {
  printf '%s' "$1" | sed -e 's/&/\&amp;/g' -e 's/</\&lt;/g' -e 's/>/\&gt;/g'
}
# CFBundleShortVersionString / CFBundleVersion must be numeric (digits + dots).
# Untagged workflow_dispatch builds use `dev-<sha>`; map those to 0.0.0.
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
	<string>spec_chum</string>
	<key>CFBundleIdentifier</key>
	<string>dev.specchum.spec-chum</string>
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
	<string>11.0</string>
	<key>NSHighResolutionCapable</key>
	<true/>
	<key>NSPrincipalClass</key>
	<string>NSApplication</string>
</dict>
</plist>
PLIST

# PkgInfo is optional but conventional for APPL bundles.
printf 'APPL????' > "$APP_DST/Contents/PkgInfo"

echo "staged $APP_DST (egui Spec Chum.app, version ${VERSION})"
