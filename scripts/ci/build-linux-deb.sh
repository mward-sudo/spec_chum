#!/usr/bin/env bash
# Build a Spec Chum .deb from a staged release tree (Refs #231).
#
# Usage:
#   build-linux-deb.sh <version> <stage-dir> <output.deb>
#
# Expects stage-dir to contain: spec_chum, LICENSE, README.txt.
# Desktop entry + icon come from packaging/linux/ beside this scripts/ci tree
# (release CI invokes this from the trusted default-branch checkout — CWE-829).
#
# Installs to /usr/bin/spec_chum with a .desktop entry and hicolor icon.
# Declares runtime Depends matching the release README (GTK 3, ALSA, udev).
set -euo pipefail

if [[ $# -ne 3 ]]; then
  echo "usage: $0 <version> <stage-dir> <output.deb>" >&2
  exit 2
fi

VERSION="$1"
STAGE_DIR="$2"
OUT_DEB="$3"

# Debian upstream version: digits and dots preferred; allow the same safe set
# as other packaging scripts (dev-<sha> workflow_dispatch builds included).
if [[ ! "$VERSION" =~ ^[0-9A-Za-z][0-9A-Za-z._-]*$ ]]; then
  echo "error: refusing unsafe version string: $VERSION" >&2
  exit 1
fi

BIN="$STAGE_DIR/spec_chum"
LICENSE="$STAGE_DIR/LICENSE"
README="$STAGE_DIR/README.txt"
for required in "$BIN" "$LICENSE" "$README"; do
  if [[ ! -f "$required" ]]; then
    echo "error: staged release tree missing required file: $required" >&2
    exit 1
  fi
done
if [[ ! -x "$BIN" ]]; then
  echo "error: staged binary is not executable: $BIN" >&2
  exit 1
fi

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
DESKTOP="$REPO_ROOT/packaging/linux/spec-chum.desktop"
ICON="$REPO_ROOT/packaging/linux/spec-chum.png"
if [[ ! -f "$DESKTOP" || ! -f "$ICON" ]]; then
  echo "error: packaging/linux assets missing beside trusted CI tree" >&2
  exit 1
fi

if [[ "${OUT_DEB##*.}" != "deb" ]]; then
  echo "error: output path must end with .deb: $OUT_DEB" >&2
  exit 1
fi

if ! command -v dpkg-deb >/dev/null 2>&1; then
  echo "error: dpkg-deb not found (install dpkg)" >&2
  exit 1
fi

TMP="${RUNNER_TEMP:-${TMPDIR:-/tmp}}/spec-chum-deb-$$"
ROOT="$TMP/root"
mkdir -p \
  "$ROOT/DEBIAN" \
  "$ROOT/usr/bin" \
  "$ROOT/usr/share/applications" \
  "$ROOT/usr/share/icons/hicolor/256x256/apps" \
  "$ROOT/usr/share/doc/spec-chum"
cleanup() {
  rm -rf "$TMP"
}
trap cleanup EXIT

cp "$BIN" "$ROOT/usr/bin/spec_chum"
chmod 755 "$ROOT/usr/bin/spec_chum"
cp "$DESKTOP" "$ROOT/usr/share/applications/spec-chum.desktop"
cp "$ICON" "$ROOT/usr/share/icons/hicolor/256x256/apps/spec-chum.png"
cp "$LICENSE" "$ROOT/usr/share/doc/spec-chum/copyright"
cp "$README" "$ROOT/usr/share/doc/spec-chum/README.txt"
gzip -9n -c "$README" >"$ROOT/usr/share/doc/spec-chum/README.txt.gz"
rm -f "$ROOT/usr/share/doc/spec-chum/README.txt"

# Installed-Size in KiB (Debian Policy §5.6.20) — exclude DEBIAN/.
INSTALLED_SIZE="$(du -sk "$ROOT/usr" | awk '{print $1}')"

# libasound2t64 is the Ubuntu 24.04+ package name; keep libasound2 for older.
cat >"$ROOT/DEBIAN/control" <<EOF
Package: spec-chum
Version: ${VERSION}
Section: games
Priority: optional
Architecture: amd64
Installed-Size: ${INSTALLED_SIZE}
Depends: libgtk-3-0, libasound2t64 | libasound2, libudev1, libc6
Maintainer: Spec Chum <https://github.com/mward-sudo/spec_chum/issues>
Homepage: https://github.com/mward-sudo/spec_chum
Description: Hardware-accurate ZX Spectrum emulator
 Spec Chum is a from-scratch ZX Spectrum emulator (egui host). Headless
 debugger and agent HTTP live on the same binary (spec_chum --serve /
 spec_chum debug …). System ROMs are not included.
EOF

mkdir -p "$(dirname "$OUT_DEB")"
rm -f "$OUT_DEB"
dpkg-deb --root-owner-group --build "$ROOT" "$OUT_DEB"

if [[ ! -f "$OUT_DEB" ]]; then
  echo "error: dpkg-deb did not produce $OUT_DEB" >&2
  exit 1
fi
echo "created $OUT_DEB"
