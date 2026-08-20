#!/usr/bin/env bash
# Fetch third-party ULA/timing TAP fixtures into .rom-cache (not git).
# Re-downloads only when a file is missing, unless FORCE_SYSTEM_TESTS=1.
set -euo pipefail
cd "$(dirname "$0")/.."
DEST=.rom-cache/system-tests
mkdir -p "$DEST"

FORCE="${FORCE_SYSTEM_TESTS:-0}"

fetch() {
  local url=$1 out=$2
  if [[ -f "$out" && "$FORCE" != 1 ]]; then
    echo "cached $(basename "$out")"
    return
  fi
  echo "fetch $url"
  curl -fsSL -o "$out" "$url"
}

fetch "http://torinak.com/~jb/zx/minfo.tap" "$DEST/minfo.tap"
fetch "http://torinak.com/~jb/zx/ulatest3.tap" "$DEST/ulatest3.tap"

ZIP=.rom-cache/timingtest-0.3.zip
fetch "http://zxds.raxoft.cz/taps/misc/timingtest.zip" "$ZIP"
if [[ ! -f "$DEST/timingtest.tap" || "$FORCE" == 1 ]]; then
  TMP=$(mktemp -d)
  trap 'rm -rf "$TMP"' EXIT
  unzip -qo "$ZIP" -d "$TMP"
  cp -f "$TMP"/timingtest-0.3/timing.tap "$DEST/timingtest.tap"
  rm -rf "$TMP"
  trap - EXIT
fi

echo "System-test TAPs in $DEST (gitignored via .rom-cache/)"
ls -la "$DEST"/*.tap
