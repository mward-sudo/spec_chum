# +3DOS / Loader DSK smokes

Redistributable fixtures live **in code** (`DskImage::synthetic_plus3_*` in
`crates/formats/src/dsk.rs`), not as `.dsk` blobs in git. ROM-gated tests skip
when `roms/plus3/plus3.rom` is missing (`./scripts/fetch_roms.sh`).

The +3 menu **Loader** path (same as Fuse’s phantom typist pressing Enter, and
the +3 manual chapter 8 parts 26–27):

1. `DOS_BOOT` — read track 0 sector 1; if the 512-byte sum is **3 mod 256**,
   jump to `FE10h` (commercial titles, DSKTOOL/VTAPE, zx3-drive-tester).
2. Else `LOAD "DISK"` — +3DOS file named `DISK` (john_e / retrocomputing.SE).
3. Else tape `LOAD ""`.

| Fixture | Licence | Loader path | Marker |
| --- | --- | --- | --- |
| `synthetic_plus3_data` | in-repo | FDC READ/SEEK only | `read_count` / `seek_count` |
| `synthetic_plus3_boot_marker` | in-repo | `DOS_BOOT` | border 2 or `FE20=A5` |
| `synthetic_plus3_disk_basic` | in-repo | `LOAD "DISK"` → BASIC `RUN` | `POKE 32768,165` |

Disk geometry follows seasip PCW 180K / zx3dsk: 40T SS 9×512 (ids 1–9), 1K
blocks, 2 directory blocks, reserved track `OFF=1` (directory at track 1).

## Not in git (optional local titles)

Do not commit commercial DSKs. Fuse’s informal +3 soak list (Chase H.Q.,
Gauntlet II, …) is copyrighted; use a local dump if you want a titled soak
outside CI. Public-domain +3 images may be added later via a fetch script
(same pattern as `tests/fixtures/system/`).

## CP/M SYSTEM

Not covered. A CP/M Plus boot disk needs a copyrighted or separately licensed
system image; note only. `DOS_BOOT` with checksum 3 is the hook those disks
use — the titled bootstrap smoke exercises that entry without shipping CP/M.

## Unsupported µPD765 commands

SCAN EQUAL (`0x11`), SCAN LOW OR EQUAL (`0x19`), SCAN HIGH OR EQUAL (`0x1d`),
and READ TRACK (`0x02`) — including MT/SK variants — return invalid-command
`ST0=0x80` immediately (see `Plus3Fdc` module docs / unit tests). No +3DOS or
Loader path issues them; FORMAT TRACK is implemented. Copy-protected /
non-standard DSK geometry is not modelled.
