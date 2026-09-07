#!/usr/bin/env bash
# Build a Spec Chum AppImage from a staged release tree (Refs #231).
#
# Usage:
#   build-linux-appimage.sh <version> <stage-dir> <output.AppImage>
#
# Expects stage-dir to contain: spec_chum, LICENSE, README.txt.
# Desktop entry + icon come from packaging/linux/ beside this scripts/ci tree
# (release CI invokes this from the trusted default-branch checkout — CWE-829).
#
# Runtime still needs GTK 3 + ALSA + udev on the host (same as the .tar.gz);
# this wraps the single primary binary for double-click / PATH-free use.
set -euo pipefail

if [[ $# -ne 3 ]]; then
  echo "usage: $0 <version> <stage-dir> <output.AppImage>" >&2
  exit 2
fi

VERSION="$1"
STAGE_DIR="$2"
OUT_APPIMAGE="$3"

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

if [[ "${OUT_APPIMAGE##*.}" != "AppImage" ]]; then
  echo "error: output path must end with .AppImage: $OUT_APPIMAGE" >&2
  exit 1
fi

# Pinned appimagetool 1.9.1 (x86_64). Bump URL + SHA together when upgrading.
APPIMAGETOOL_URL="https://github.com/AppImage/appimagetool/releases/download/1.9.1/appimagetool-x86_64.AppImage"
APPIMAGETOOL_SHA256="ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0"

TMP="${RUNNER_TEMP:-${TMPDIR:-/tmp}}/spec-chum-appimage-$$"
APPDIR="$TMP/SpecChum.AppDir"
TOOL_DIR="$TMP/tools"
mkdir -p "$APPDIR/usr/bin" "$APPDIR/usr/share/applications" \
  "$APPDIR/usr/share/icons/hicolor/256x256/apps" \
  "$APPDIR/usr/share/doc/spec-chum" "$TOOL_DIR"
cleanup() {
  rm -rf "$TMP"
}
trap cleanup EXIT

cp "$BIN" "$APPDIR/usr/bin/spec_chum"
chmod 755 "$APPDIR/usr/bin/spec_chum"
cp "$LICENSE" "$README" "$APPDIR/usr/share/doc/spec-chum/"
cp "$DESKTOP" "$APPDIR/usr/share/applications/spec-chum.desktop"
cp "$DESKTOP" "$APPDIR/spec-chum.desktop"
cp "$ICON" "$APPDIR/usr/share/icons/hicolor/256x256/apps/spec-chum.png"
cp "$ICON" "$APPDIR/spec-chum.png"

cat > "$APPDIR/AppRun" << 'EOF'
#!/usr/bin/env bash
set -euo pipefail
HERE="$(dirname "$(readlink -f "$0")")"
exec "$HERE/usr/bin/spec_chum" "$@"
EOF
chmod 755 "$APPDIR/AppRun"

TOOL="$TOOL_DIR/appimagetool-x86_64.AppImage"
curl -fsSL --retry 3 --retry-delay 2 -o "$TOOL" "$APPIMAGETOOL_URL"
got="$(sha256sum "$TOOL" | awk '{print $1}')"
if [[ "$got" != "$APPIMAGETOOL_SHA256" ]]; then
  echo "error: appimagetool sha256 mismatch" >&2
  echo "  expected: $APPIMAGETOOL_SHA256" >&2
  echo "  got:      $got" >&2
  exit 1
fi
chmod +x "$TOOL"

mkdir -p "$(dirname "$OUT_APPIMAGE")"
rm -f "$OUT_APPIMAGE"

# GHA runners often lack FUSE; extract-and-run avoids mounting the tool.
export ARCH=x86_64
export APPIMAGE_EXTRACT_AND_RUN=1
export VERSION
"$TOOL" "$APPDIR" "$OUT_APPIMAGE"

if [[ ! -f "$OUT_APPIMAGE" ]]; then
  echo "error: appimagetool did not produce $OUT_APPIMAGE" >&2
  exit 1
fi
chmod +x "$OUT_APPIMAGE"
echo "created $OUT_APPIMAGE"
