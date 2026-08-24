#!/usr/bin/env bash
# Fetch third-party ULA/timing TAP fixtures into .rom-cache (not git).
# Re-downloads only when a file is missing, unless FORCE_SYSTEM_TESTS=1.
# Each artefact is written atomically and checked against a committed SHA-256.
set -euo pipefail
cd "$(dirname "$0")/.."
DEST=.rom-cache/system-tests
mkdir -p "$DEST"

FORCE="${FORCE_SYSTEM_TESTS:-0}"

# Prefer HTTPS when the host serves it; raxoft ZIP is HTTP-only today.
SHA_MINFO_TAP="c1ff004f9a5cb66d99afadff618c59e255e19ad33bd87908e63977c864d4979d"
SHA_ULATEST3_TAP="9445d3bd1661c2d5a62e2b3762ebd1ab00af9b319435ae7397bf6eb51462c6c9"
SHA_TIMINGTEST_ZIP="bacff01453a01c14754c167b5b02695ec29a0a5960c2f202b52b26494c2e8dff"
SHA_TIMINGTEST_TAP="da62ba6438af1398b3e1d1bbd3627985373b8cc81a72628421285329f4903962"

sha256_of() {
  local path=$1
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$path" | awk '{print $1}'
  else
    sha256sum "$path" | awk '{print $1}'
  fi
}

verify_sha() {
  local path=$1 expected=$2
  local actual
  actual="$(sha256_of "$path")"
  if [[ "$actual" != "$expected" ]]; then
    echo "SHA-256 mismatch for $(basename "$path"): got $actual want $expected" >&2
    rm -f "$path"
    return 1
  fi
}

fetch() {
  local url=$1 out=$2 expected=$3
  if [[ -f "$out" && "$FORCE" != 1 ]]; then
    if verify_sha "$out" "$expected"; then
      echo "cached $(basename "$out")"
      return
    fi
    echo "cache invalid for $(basename "$out"); re-fetching"
  fi
  local tmp="${out}.tmp.$$"
  rm -f "$tmp"
  echo "fetch $url"
  if ! curl -fsSL -o "$tmp" "$url"; then
    rm -f "$tmp"
    echo "download failed: $url" >&2
    return 1
  fi
  if ! verify_sha "$tmp" "$expected"; then
    rm -f "$tmp"
    return 1
  fi
  mv -f "$tmp" "$out"
}

fetch "https://torinak.com/~jb/zx/minfo.tap" "$DEST/minfo.tap" "$SHA_MINFO_TAP"
fetch "https://torinak.com/~jb/zx/ulatest3.tap" "$DEST/ulatest3.tap" "$SHA_ULATEST3_TAP"

ZIP=.rom-cache/timingtest-0.3.zip
fetch "http://zxds.raxoft.cz/taps/misc/timingtest.zip" "$ZIP" "$SHA_TIMINGTEST_ZIP"
if [[ ! -f "$DEST/timingtest.tap" || "$FORCE" == 1 ]]; then
  TMP=$(mktemp -d)
  trap 'rm -rf "$TMP"' EXIT
  unzip -qo "$ZIP" -d "$TMP"
  cp -f "$TMP"/timingtest-0.3/timing.tap "$DEST/timingtest.tap.tmp.$$"
  if ! verify_sha "$DEST/timingtest.tap.tmp.$$" "$SHA_TIMINGTEST_TAP"; then
    rm -f "$DEST/timingtest.tap.tmp.$$"
    rm -rf "$TMP"
    trap - EXIT
    exit 1
  fi
  mv -f "$DEST/timingtest.tap.tmp.$$" "$DEST/timingtest.tap"
  rm -rf "$TMP"
  trap - EXIT
elif ! verify_sha "$DEST/timingtest.tap" "$SHA_TIMINGTEST_TAP"; then
  echo "cached timingtest.tap failed digest check; re-run with FORCE_SYSTEM_TESTS=1" >&2
  exit 1
fi

# Floating Spy v0.33 (Ramsoft) — zip contains floatspy.tap
SHA_FLOATSPY_ZIP="3663cfc76b0733491c69faf088dfacda9294aa76e828600070380086138747ed"
SHA_FLOATSPY_TAP="dc4a3ba0b0b74396919e0a67f0984aaa5762a3bdd3e0afb4bc38ac72fa7bef34"
FLOAT_ZIP=.rom-cache/floating-spy-0.33.zip
fetch "https://zxe.io/depot/software/ZX%20Spectrum/Floating%20Spy%20v0.33%20%282002-04%29%28Ramsoft%29%5B%21%5D.zip" \
  "$FLOAT_ZIP" "$SHA_FLOATSPY_ZIP"
if [[ ! -f "$DEST/floatspy.tap" || "$FORCE" == 1 ]]; then
  TMP=$(mktemp -d)
  trap 'rm -rf "$TMP"' EXIT
  unzip -qo "$FLOAT_ZIP" -d "$TMP"
  cp -f "$TMP"/floatspy.tap "$DEST/floatspy.tap.tmp.$$"
  if ! verify_sha "$DEST/floatspy.tap.tmp.$$" "$SHA_FLOATSPY_TAP"; then
    rm -f "$DEST/floatspy.tap.tmp.$$"
    rm -rf "$TMP"
    trap - EXIT
    exit 1
  fi
  mv -f "$DEST/floatspy.tap.tmp.$$" "$DEST/floatspy.tap"
  rm -rf "$TMP"
  trap - EXIT
elif ! verify_sha "$DEST/floatspy.tap" "$SHA_FLOATSPY_TAP"; then
  echo "cached floatspy.tap failed digest check; re-extracting from zip"
  rm -f "$DEST/floatspy.tap"
  TMP=$(mktemp -d)
  trap 'rm -rf "$TMP"' EXIT
  unzip -qo "$FLOAT_ZIP" -d "$TMP"
  cp -f "$TMP"/floatspy.tap "$DEST/floatspy.tap.tmp.$$"
  if ! verify_sha "$DEST/floatspy.tap.tmp.$$" "$SHA_FLOATSPY_TAP"; then
    rm -f "$DEST/floatspy.tap.tmp.$$"
    rm -rf "$TMP"
    trap - EXIT
    exit 1
  fi
  mv -f "$DEST/floatspy.tap.tmp.$$" "$DEST/floatspy.tap"
  rm -rf "$TMP"
  trap - EXIT
fi

# azesmbog ULA TAP suite (visual / timing — load + paint smoke)
SHA_ULA48_SIMPLE="1bcd04d0dda815eb8ae49014b828752288551f596a82c7ffd46662d6f82f2c4e"
SHA_ULA128_TIMING="59578bae6352d6a92b1887b392ee786aa9f162624a1ddef9541037b517b0c90f"
SHA_ULA128E_PLUS3="60b3aaeca5b9d45c712d874fafa136c97d373c1569964cb87703ddf53911a8d5"
ZXE="https://zxe.io/depot/software/ZX%20Spectrum"
fetch "$ZXE/ULA%2048%20Simple%20Test%20%282012-10-06%29%28azesmbog%29%5B%21%5D.tap" \
  "$DEST/ula48_simple.tap" "$SHA_ULA48_SIMPLE"
fetch "$ZXE/ULA%20128%20Timing%20Test%20%282012-10-06%29%28azesmbog%29%5B%21%5D.tap" \
  "$DEST/ula128_timing.tap" "$SHA_ULA128_TIMING"
fetch "$ZXE/ULA%20128E%20%2B3%20Test%20%282012-10-10%29%28azesmbog%29%5B%21%5D.tap" \
  "$DEST/ula128e_plus3.tap" "$SHA_ULA128E_PLUS3"

echo "System-test TAPs in $DEST (gitignored via .rom-cache/)"
ls -la "$DEST"/*.tap
