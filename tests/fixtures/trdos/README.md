# TR-DOS / Beta Disk smokes

Redistributable fixtures live **in code** (`TrdImage::synthetic_trdos_boot_basic` in
`crates/formats/src/trd.rs`), not as `.trd` blobs in git. ROM-gated tests skip
when `roms/trdos.rom` is missing (user-provided — **not** `./scripts/fetch_roms.sh`;
see [ROMS.md](../../../docs/ROMS.md)).

Always-on CI reads the `boot` file body through VG93 (and a synthetic TR-DOS
`IN A,(#FF)` / `INI` loop). That is the filesystem + FDC slice of
[#140](https://github.com/mward-sudo/spec_chum/issues/140), not a real DOS `RUN`.
egui and SpecChumMac share attach / Open TRD / Load TR-DOS ROM (`sc_attach_beta`,
`sc_load_trd`, `sc_load_trdos_rom`; agent `POST /v1/hardware/beta` and `/v1/trd`).

**VG93 WRITE TRACK (`0xE0`/`0xF0`):** implemented in `crates/bus/src/beta_disk.rs`
— parses IBM-style ID (`FE` + C/H/R/N) and data (`FB` + 256 bytes) streams from
TR-DOS `NEW` / format, commits sectors into `TrdImage`, auto-completes after 16
sectors/track. Always-on smokes: `write_track_formats_sector_with_data`,
`write_track_auto_completes_full_track`, `beta_write_track_via_synthetic_rom`.

**Beta 128 entry:** TR-DOS 5.04 compares `(PROG)` against `5D25h` at `3D21h` (not
`(CHANS)`). ROM-gated machine tests boot 128 BASIC from the menu, patch `(PROG)` into
the 128K workspace when needed, and enter via `USR 15616` (`3D00h`). The Beta paging
latch in `crates/bus/src/beta_disk.rs` stays set across RAM execution so mixed
ROM/RAM warm-boot paths can resume below `4000h` without re-entering through
`3D00–3DFF`.

| ROM-gated test | When `trdos.rom` present |
| --- | --- |
| `trdos_rom_reads_boot_when_128k_chans_ok_and_fixture_present` | After DOS entry, VG93 can still read track 1 / sector 1 (`boot` body) |
| `trdos_rom_run_boot_basic_when_fixture_present` | **Open:** `RUN` → `POKE 32768,165` (`0x8000 == 0xA5`); soft-skips when `roms/trdos.rom` present until TR-DOS seek/catalog completes |

`RUN` with no filename loads the BASIC program named `boot` (Beta 128 manual).

| Fixture | Licence | Path | Marker |
| --- | --- | --- | --- |
| `synthetic_trdos_boot_basic` | in-repo | VG93/CPU read of BASIC `boot` | `POKE 32768,165` |

Geometry: 40-track SS, 16×256 (`disk_type = 0x19`). Directory + volume info on
track 0; `boot` body at track 1, TR-DOS start sector 0 (VG93 sector ID 1).

## Not in git

Do not commit TR-DOS ROM dumps (© Technology Research Ltd). Place a 16 KiB
image at `roms/trdos.rom`, `roms/trdos/trdos.rom`, or `roms/pentagon/trdos.rom`.
Commercial `.trd` titles stay local; do not add them to this tree.
