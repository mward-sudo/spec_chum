#!/usr/bin/env bash
# Build (if needed) and launch the native macOS Spec Chum app via Launch Services.
#
# Prefer `open` on a staged .app bundle so SpecChum becomes the key application.
# Do not `exec` the raw SwiftPM binary as a Terminal child — that leaves Terminal
# key and keystrokes echo in the shell instead of driving Spectrum BASIC.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="$ROOT/apps/macos/.build/release/SpecChumMac"
ROOM_A="$ROOT/target/release/libspec_chum_room.a"
APP="$ROOT/apps/macos/.build/SpecChumMac.app"
CONTENTS="$APP/Contents"
MACOS_DIR="$CONTENTS/MacOS"

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

if [[ ! -x "$BIN" || ! -f "$ROOM_A" ]]; then
  "$ROOT/scripts/build_macos_app.sh"
fi

mkdir -p "$MACOS_DIR" "$CONTENTS/Resources"

# Real binary (wrapper sets env then execs this).
cp -f "$BIN" "$MACOS_DIR/SpecChumMac.bin"
chmod +x "$MACOS_DIR/SpecChumMac.bin"

# Launcher so SPEC_CHUM_ROOT / asset env survive Launch Services `open`.
# Bake living-room / perf flags into the wrapper when set at stage time — `open`
# does not forward the parent shell environment into the app.
cat > "$MACOS_DIR/SpecChumMac" <<EOF
#!/bin/bash
set -euo pipefail
export SPEC_CHUM_ROOT="$ROOT"
export SPEC_CHUM_LIVING_ROOM_ASSETS="$CONTENTS/Resources/living_room_assets"
EOF
if [[ -n "${SPEC_CHUM_LIVING_ROOM:-}" ]]; then
  printf 'export SPEC_CHUM_LIVING_ROOM=%q\n' "$SPEC_CHUM_LIVING_ROOM" >> "$MACOS_DIR/SpecChumMac"
fi
if [[ -n "${SPEC_CHUM_ROOM_PERF:-}" ]]; then
  printf 'export SPEC_CHUM_ROOM_PERF=%q\n' "$SPEC_CHUM_ROOM_PERF" >> "$MACOS_DIR/SpecChumMac"
fi
if [[ -n "${SPEC_CHUM_ROOM_APERTURE_DEBUG:-}" ]]; then
  printf 'export SPEC_CHUM_ROOM_APERTURE_DEBUG=%q\n' "$SPEC_CHUM_ROOM_APERTURE_DEBUG" >> "$MACOS_DIR/SpecChumMac"
fi
if [[ -n "${SPEC_CHUM_ROOM_HIDE_CRT:-}" ]]; then
  printf 'export SPEC_CHUM_ROOM_HIDE_CRT=%q\n' "$SPEC_CHUM_ROOM_HIDE_CRT" >> "$MACOS_DIR/SpecChumMac"
fi
if [[ -n "${SPEC_CHUM_ROOM_BRIGHT_DEBUG:-}" ]]; then
  printf 'export SPEC_CHUM_ROOM_BRIGHT_DEBUG=%q\n' "$SPEC_CHUM_ROOM_BRIGHT_DEBUG" >> "$MACOS_DIR/SpecChumMac"
fi
if [[ -n "${SPEC_CHUM_AUDIO_DEBUG:-}" ]]; then
  printf 'export SPEC_CHUM_AUDIO_DEBUG=%q\n' "$SPEC_CHUM_AUDIO_DEBUG" >> "$MACOS_DIR/SpecChumMac"
fi
if [[ -n "${SPEC_CHUM_INPUT_LATENCY:-}" ]]; then
  printf 'export SPEC_CHUM_INPUT_LATENCY=%q\n' "$SPEC_CHUM_INPUT_LATENCY" >> "$MACOS_DIR/SpecChumMac"
fi
cat >> "$MACOS_DIR/SpecChumMac" <<EOF
cd "$ROOT"
exec "\$(dirname "\$0")/SpecChumMac.bin" "\$@"
EOF
chmod +x "$MACOS_DIR/SpecChumMac"

# Ensure assets are present in the staged app (shared with build_macos_app.sh).
"$ROOT/scripts/stage_living_room_assets.sh" "$ROOT" "$CONTENTS/Resources/living_room_assets"

cat > "$CONTENTS/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleDevelopmentRegion</key>
	<string>en</string>
	<key>CFBundleExecutable</key>
	<string>SpecChumMac</string>
	<key>CFBundleIdentifier</key>
	<string>dev.specchum.SpecChumMac</string>
	<key>CFBundleInfoDictionaryVersion</key>
	<string>6.0</string>
	<key>CFBundleName</key>
	<string>Spec Chum</string>
	<key>CFBundlePackageType</key>
	<string>APPL</string>
	<key>CFBundleShortVersionString</key>
	<string>0.2.0</string>
	<key>CFBundleVersion</key>
	<string>1</string>
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

echo "Launching $APP (click the Spec Chum window; do not type in this Terminal)."
open "$APP"
