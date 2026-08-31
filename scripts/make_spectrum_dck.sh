#!/usr/bin/env bash
# Build a Warajevo/Fuse Spectrum ROM .dck from fetched roms/spec48.rom.
# Does NOT commit or redistribute ROM bytes — output is local only.
#
# Usage:
#   ./scripts/make_spectrum_dck.sh home [out.dck]   # HOME replace (Timex → Spectrum)
#   ./scripts/make_spectrum_dck.sh dock [out.dck]   # DOCK cart (page with OUT 244,3)
#
# See docs/TIMEX.md.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
MODE="${1:-}"
OUT="${2:-}"
ROM="${ROOT}/roms/spec48.rom"

usage() {
  echo "usage: $0 home|dock [output.dck]" >&2
  exit 2
}

[[ -n "$MODE" ]] || usage

if [[ ! -f "$ROM" ]]; then
  echo "error: missing $ROM — run ./scripts/fetch_roms.sh first" >&2
  exit 1
fi

size=$(wc -c < "$ROM" | tr -d ' ')
if [[ "$size" -lt 16384 ]]; then
  echo "error: $ROM too small ($size bytes; need 16384)" >&2
  exit 1
fi

case "$MODE" in
  home)
    # Bank 255 (HOME), chunks 0+1 ROM.
    HEADER_FMT='\xff\x02\x02\x00\x00\x00\x00\x00\x00'
    OUT="${OUT:-${ROOT}/roms/timex/spectrum-home.dck}"
    ;;
  dock)
    # Bank 0 (DOCK), chunks 0+1 ROM — leading 0x00 must not use %s.
    HEADER_FMT='\x00\x02\x02\x00\x00\x00\x00\x00\x00'
    OUT="${OUT:-${ROOT}/roms/timex/spectrum-dock.dck}"
    ;;
  *)
    usage
    ;;
esac

if [[ "$OUT" -ef "$ROM" ]]; then
  echo "error: output must not overwrite $ROM" >&2
  exit 2
fi

mkdir -p "$(dirname "$OUT")"
# First 16 KiB only (Spectrum ROM); ignore any longer dumps.
{
  # shellcheck disable=SC2059
  printf "$HEADER_FMT"
  head -c 16384 "$ROM"
} >"$OUT"

bytes=$(wc -c < "$OUT" | tr -d ' ')
if [[ "$bytes" -ne 16393 ]]; then
  echo "error: expected 16393-byte .dck, got $bytes" >&2
  exit 1
fi

echo "wrote $OUT ($bytes bytes)"
