#!/usr/bin/env bash
# Copy managed redistributable ROMs into a packaging destination.
#
# Usage:
#   bundle-release-roms.sh <repo-root-with-fetched-roms> <dest-parent>
#
# Writes <dest-parent>/roms/ (same layout as ./scripts/fetch_roms.sh) plus
# ROMS-NOTICE.txt with the Lawson 1999 attribution. Never commits ROM bytes —
# release CI fetches first, then embeds into app bundles / installers.
#
# See docs/ROMS.md. Copies only the exact complete Spectrum Next distribution
# archive; the separate GPL IPL is acquired from its official source on user
# request. Other user-provided-only firmware trees remain excluded.
set -euo pipefail

if [[ $# -ne 2 ]]; then
  echo "usage: $0 <repo-root-with-fetched-roms> <dest-parent>" >&2
  exit 2
fi

ROOT="$(cd "$1" && pwd)"
DEST_PARENT="$(mkdir -p "$2" && cd "$2" && pwd)"
SRC="$ROOT/roms"
DEST="$DEST_PARENT/roms"

if [[ ! -f "$SRC/spec48.rom" ]]; then
  echo "error: missing $SRC/spec48.rom — run ./scripts/fetch_roms.sh first" >&2
  exit 1
fi

rm -rf "$DEST"
mkdir -p "$DEST"

# Managed fetch inventory only (docs/ROMS.md). Do not copy user dumps.
copy_tree() {
  local rel="$1"
  local from="$SRC/$rel"
  local to="$DEST/$rel"
  if [[ ! -e "$from" ]]; then
    echo "error: missing managed ROM path: $from" >&2
    exit 1
  fi
  mkdir -p "$(dirname "$to")"
  if [[ -d "$from" ]]; then
    mkdir -p "$to"
    # Copy .rom (+ SpeccyBoot LICENSE) only; skip unrelated files.
    find "$from" -maxdepth 1 \( -name '*.rom' -o -name 'LICENSE' \) -type f | while IFS= read -r f; do
      cp -f "$f" "$to/$(basename "$f")"
    done
  else
    cp -f "$from" "$to"
  fi
}

copy_tree "spec48.rom"
for d in alternate 128 plus2 plus2a plus3 fuse-16k timex opense plus3e \
  peripherals/datel peripherals/speccyboot; do
  copy_tree "$d"
done

if [[ -f "$ROOT/scripts/system_next_assets.py" ]]; then
  python3 "$ROOT/scripts/system_next_assets.py" --verify
  NEXT_SOURCE="$SRC/system-next/24.11/sn-complete-24.11.zip"
  NEXT_DEST="$DEST/system-next/24.11"
  mkdir -p "$NEXT_DEST"
  cp -f "$NEXT_SOURCE" "$NEXT_DEST/sn-complete-24.11.zip"
  cat > "$DEST_PARENT/NEXT-NOTICE.txt" <<'EOF'
ZX Spectrum Next assets in Spec Chum
=====================================

This no-fee package includes an exact copy of the complete official System/Next
Distribution 24.11 archive at roms/system-next/24.11/sn-complete-24.11.zip.
Its unmodified LICENSE.md and docs/licenses/ notices remain inside the archive.
Source: https://www.specnext.com/distro/24.11/sn-complete-24.11.zip
SHA-256: cbf5d4c8bb6dc552a4e68317a7315e06609b14028e1f04dd0afd2189be65ce6b

The separate GPL-3.0-or-later boot code is not bundled. Selecting "Get official
assets" in Spec Chum fetches the pinned boot archive from
https://www.specnext.com/forum/download/file.php?id=1164, verifies its hash,
and stores boot-30204.bin, GPL3-LICENSE, and source details in per-user data.
The archive README links upstream source reference
https://gitlab.com/SpectrumNext/ZX_Spectrum_Next_FPGA/-/tree/master/cores/zxnext/src/rom?ref_type=heads;
exact source correspondence for boot-30204.bin is unverified.
Spec Chum is unaffiliated with SpecNext Ltd and uses its own branding.
EOF
fi

# Refs help verify the pinned fetch set inside the artifact.
if [[ -f "$SRC/.zx-roms-ref" ]]; then
  cp -f "$SRC/.zx-roms-ref" "$DEST/.zx-roms-ref"
fi
if [[ -f "$SRC/.fuse-roms-ref" ]]; then
  cp -f "$SRC/.fuse-roms-ref" "$DEST/.fuse-roms-ref"
fi

cat > "$DEST_PARENT/ROMS-NOTICE.txt" <<'EOF'
Amstrad Spectrum ROM images
===========================

Amstrad have kindly given their permission for the redistribution of their
copyrighted material but retain that copyright.

Cliff Lawson (Amstrad plc, 1999-08-31) stated that Amstrad are happy for
emulator writers to include images of their copyrighted Spectrum ROM code as
long as the (c)opyright messages inside the images are not altered.

Do not strip or patch the copyright strings inside .rom files.

Additional redistributable images in this tree (Timex, OpenSE BASIC GPL-2+,
+3e, Datel +D/DISCiPLE, SpeccyBoot MIT) are documented in docs/ROMS.md.
User-provided-only firmware (Interface 1, Multiface, TR-DOS, Pentagon, …)
is not bundled.

Fetch pins: see roms/.zx-roms-ref and roms/.fuse-roms-ref when present.
EOF

count="$(find "$DEST" -type f -name '*.rom' | wc -l | tr -d ' ')"
if [[ "$count" -lt 40 ]]; then
  echo "error: expected ≥40 managed .rom files under $DEST, found $count" >&2
  exit 1
fi

echo "bundled $count ROM files → $DEST"
