#!/usr/bin/env bash
# Fetch distributable ZX Spectrum ROM images into roms/.
# ROMs are NOT redistributed by this project (not in git / not in Releases).
#
# Amstrad official Spectrum ROMs — Lawson 1999 grant; see docs/ROMS.md.
# Non-Amstrad sets below cite their own grants in docs/ROMS.md and here.
#
# Explicitly NOT fetched (user-provided only): IF1, Multiface, TR-DOS, Pentagon,
# Scorpion, Opus, ESXDOS, ZX80/81 — see docs/ROMS.md and #190.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ROM_DIR="${ROOT}/roms"
CACHE="${ROOT}/.rom-cache"

# Pinned for reproducibility (update deliberately when refreshing ROMs).
ZX_ROMS_REF="${ZX_ROMS_REF:-720ee29bc699393275d799d0d5adc4b31210b127}"
FUSE_ROMS_REF="${FUSE_ROMS_REF:-3e3fb40d29b442a495c5d3efe5cca83ace05bd38}"
ZX_ROMS_URL="https://github.com/spectrumforeveryone/zx-roms.git"
FUSE_URL="https://github.com/fuse-emulator/fuse.git"

mkdir -p "$ROM_DIR"

copy_rom() {
  local src="$1" dest="$2"
  if [[ ! -f "$src" ]]; then
    echo "error: missing $src" >&2
    return 1
  fi
  mkdir -p "$(dirname "$dest")"
  cp -f "$src" "$dest"
  if ! cmp -s "$src" "$dest"; then
    echo "error: verify failed ${dest#"$ROM_DIR"/}" >&2
    return 1
  fi
  echo "  ok ${dest#"$ROM_DIR"/} ($(wc -c < "$dest" | tr -d ' ') bytes)"
}

copy_dir_roms() {
  local src_dir="$1" dest_dir="$2"
  local copied=0
  if [[ ! -d "$src_dir" ]]; then
    echo "warn: missing directory $src_dir" >&2
    return 1
  fi
  mkdir -p "$dest_dir"
  while IFS= read -r f; do
    copy_rom "$f" "$dest_dir/$(basename "$f")"
    copied=$((copied + 1))
  done < <(find "$src_dir" -maxdepth 1 -name '*.rom' | sort)
  echo "  ${copied} file(s) in ${dest_dir#"$ROM_DIR"/}/"
}

checkout_sparse_repo() {
  local url="$1" name="$2" ref="$3"
  shift 3
  local -a sparse_paths=("$@")

  echo "Fetching ${name} @ ${ref}"
  if [[ ! -d "$CACHE/$name/.git" ]]; then
    git clone --filter=blob:none --sparse "$url" "$CACHE/$name"
  fi
  (
    cd "$CACHE/$name"
    git fetch --depth 1 origin "$ref" 2>/dev/null || git fetch origin
    git checkout -q "$ref" 2>/dev/null || {
      git fetch --depth 1 origin master || git fetch --depth 1 origin main
      git checkout -q "$ref"
    }
    git sparse-checkout set "${sparse_paths[@]}"
  )
}

echo "=== Amstrad / Sinclair official (zx-roms) ==="
echo "Grant: Lawson 1999 — see docs/ROMS.md"
checkout_sparse_repo "$ZX_ROMS_URL" zx-roms "$ZX_ROMS_REF" \
  spectrum16-48 spectrum128-plus2 spectrum-plus3

echo "Installing zx-roms into ${ROM_DIR}"
copy_rom "$CACHE/zx-roms/spectrum16-48/spec48.rom" "$ROM_DIR/spec48.rom"

mkdir -p "$ROM_DIR/alternate"
while IFS= read -r f; do
  base="$(basename "$f")"
  [[ "$base" == spec48.rom ]] && continue
  copy_rom "$f" "$ROM_DIR/alternate/$base"
done < <(find "$CACHE/zx-roms/spectrum16-48" -maxdepth 1 -name '*.rom' | sort)

copy_dir_roms "$CACHE/zx-roms/spectrum128-plus2/128" "$ROM_DIR/128"
copy_dir_roms "$CACHE/zx-roms/spectrum128-plus2/plus2" "$ROM_DIR/plus2"
copy_dir_roms "$CACHE/zx-roms/spectrum-plus3/plus2a" "$ROM_DIR/plus2a"
copy_dir_roms "$CACHE/zx-roms/spectrum-plus3/plus3" "$ROM_DIR/plus3"

echo
echo "=== Non-Amstrad distributable sets (Fuse roms/) ==="
echo "Grant notes: docs/ROMS.md — Timex (Fuse README.copyright), OpenSE (GPL-2+),"
echo "+3e (Amstrad modify + Garry Lancaster), Datel (+D/DISCiPLE), SpeccyBoot (MIT)"
checkout_sparse_repo "$FUSE_URL" fuse "$FUSE_ROMS_REF" roms

FUSE_SRC="$CACHE/fuse/roms"

echo "Fuse 16 KiB bank splits (official UK machines)"
mkdir -p "$ROM_DIR/fuse-16k"
for name in 48.rom 128-0.rom 128-1.rom plus2-0.rom plus2-1.rom \
  plus3-0.rom plus3-1.rom plus3-2.rom plus3-3.rom; do
  copy_rom "$FUSE_SRC/$name" "$ROM_DIR/fuse-16k/$name"
done

echo "Timex TC2048 / TC2068 (Fuse README.copyright; tc2048 Amstrad, tc2068 Timex mods PD)"
mkdir -p "$ROM_DIR/timex"
for name in tc2048.rom tc2068-0.rom tc2068-1.rom; do
  copy_rom "$FUSE_SRC/$name" "$ROM_DIR/timex/$name"
done

echo "OpenSE BASIC (GPL-2+)"
mkdir -p "$ROM_DIR/opense"
for name in se-0.rom se-1.rom; do
  copy_rom "$FUSE_SRC/$name" "$ROM_DIR/opense/$name"
done

echo "+3e (Amstrad modify/distribute + Garry Lancaster; Fuse README.copyright)"
mkdir -p "$ROM_DIR/plus3e"
for name in plus3e-0.rom plus3e-1.rom plus3e-2.rom plus3e-3.rom; do
  copy_rom "$FUSE_SRC/$name" "$ROM_DIR/plus3e/$name"
done

echo "Datel DISCiPLE / +D (https://www.shadowmagic.org.uk/spectrum/datel.html)"
mkdir -p "$ROM_DIR/peripherals/datel"
for name in disciple.rom plusd.rom; do
  copy_rom "$FUSE_SRC/$name" "$ROM_DIR/peripherals/datel/$name"
done

echo "SpeccyBoot v1.4 (MIT)"
mkdir -p "$ROM_DIR/peripherals/speccyboot"
copy_rom "$FUSE_SRC/speccyboot-1.4.rom" "$ROM_DIR/peripherals/speccyboot/speccyboot-1.4.rom"
cat > "$ROM_DIR/peripherals/speccyboot/LICENSE" <<'EOF'
SpeccyBoot v1.4 ROM — MIT License

Copyright Patrick Persson

Permission is hereby granted, free of charge, to any person obtaining
a copy of this software and associated documentation files (the
"Software"), to deal in the Software without restriction, including
without limitation the rights to use, copy, modify, merge, publish,
distribute, sublicense, and/or sell copies of the Software, and to
permit persons to whom the Software is furnished to do so, subject to
the following conditions:

The above copyright notice and this permission notice shall be included
in all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS
OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF
MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT.
IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY
CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT,
TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE
SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.

Source: Fuse roms/README.copyright — https://github.com/fuse-emulator/fuse
EOF

echo
echo "=== Verify ==="
# Count only script-managed paths (user-provided ROMs may live elsewhere under roms/).
count_managed_roms() {
  local n=0
  if [[ -f "$ROM_DIR/spec48.rom" ]]; then
    n=$((n + 1))
  fi
  local d
  for d in alternate 128 plus2 plus2a plus3 fuse-16k timex opense plus3e \
    peripherals/datel peripherals/speccyboot; do
    if [[ -d "$ROM_DIR/$d" ]]; then
      n=$((n + $(find "$ROM_DIR/$d" -maxdepth 1 -name '*.rom' 2>/dev/null | wc -l | tr -d ' ')))
    fi
  done
  echo "$n"
}
EXPECTED_ROM_COUNT=40
ACTUAL="$(count_managed_roms)"
if [[ "$ACTUAL" -eq "$EXPECTED_ROM_COUNT" ]]; then
  echo "ok: ${ACTUAL} managed ROM files (expected ${EXPECTED_ROM_COUNT})"
else
  echo "warn: found ${ACTUAL} managed ROM files, expected ${EXPECTED_ROM_COUNT}" >&2
  echo "  Compare with docs/ROMS.md inventory; re-run or check upstream refs." >&2
  exit 1
fi

echo "$ZX_ROMS_REF" > "$ROM_DIR/.zx-roms-ref"
echo "$FUSE_ROMS_REF" > "$ROM_DIR/.fuse-roms-ref"

echo "Done. ROM dir: $ROM_DIR"
echo "Refs: zx-roms=${ZX_ROMS_REF} fuse=${FUSE_ROMS_REF}"
