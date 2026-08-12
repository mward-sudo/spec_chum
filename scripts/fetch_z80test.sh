#!/usr/bin/env bash
# Fetch Patrik Rak z80test TAPs (MIT) into tests/fixtures/z80test.
set -euo pipefail
cd "$(dirname "$0")/.."
VER=v1.2a
URL="https://github.com/raxoft/z80test/releases/download/${VER}/z80test-1.2a.zip"
DEST=tests/fixtures/z80test
mkdir -p "$DEST" .rom-cache
ZIP=.rom-cache/z80test-1.2a.zip
if [[ ! -f "$ZIP" ]]; then
  curl -fsSL -o "$ZIP" "$URL"
fi
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
unzip -qo "$ZIP" -d "$TMP"
cp -f "$TMP"/z80test-1.2a/*.tap "$DEST/"
cp -f "$TMP"/z80test-1.2a/license.txt "$DEST/LICENSE.txt"
echo "Installed z80test TAPs into $DEST"
ls -la "$DEST"/*.tap
