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
| `trdos_3d94_rst20_returns_without_rom_ret_patch_when_fixture_present` | Stock `3D94h` `RST #20` / `0010h` returns via RAM `5CC2h` hook (no ROM RET patch) |
| `trdos_find_boot_rom_unpatched_when_fixture_present` | Find-boot `1968h`/`1977h`/`1988h`/`199Ah` stay stock; catalog ABI is seeded at `195Ch`/`196Ah` |
| `trdos_3dff_delay_rom_unpatched_when_fixture_present` | Motor/seek delay `3DFFh` stays stock `LD C,#FF`; harness seeds `A=1` instead of a ROM RET |
| `trdos_cat_wait_rom_unpatched_when_fixture_present` | CAT/wait sites `3D9Dh`/`02D4h`/`213Eh`/`2155h` stay stock; harness skips via PC/RET ABI |
| `trdos_19ec_08d2_callsite_rom_unpatched_when_fixture_present` | Post-match `19ECh` stays stock `RST #20`/`08D2h`; `08D2h` FF padding is not written; handoff is PC ABI in `apply_trdos_run_native_abi` |
| `trdos_012a_0d6b_service_rom_unpatched_when_fixture_present` | Native `012Ah`/`1D97h` stay stock; `0D6Bh` is FF (`0800h`–`0E71h` hole); `16B0h` is mid-`CALL 166Fh`, not a service |
| `trdos_native_file_services_gate_when_fixture_present` | Soft: hole dump → document FF at `08D2h`/`0D6Bh`; complete dump (`trdos-5.04t.rom` / `trdos-complete.rom`) → assert those sites are live (native RUN eligible) |
| `trdos_19ec_skips_fdc_standin_when_complete_rom_present` | Complete dump: `19ECh` ABI must **not** jump to `1B76h` FDC stand-in |
| `trdos_rom_run_boot_native_08d2_when_complete_present` | `#[ignore]` until 5.04T find-boot `CALL 1E3Dh` returns to `1981h` (ports `#08/#28/#48/#68` + `1FEBh` ABI landed; catalog-fill loop still open) |
| `trdos_rom_run_boot_basic_when_fixture_present` | Catalog match + VG93 body load at **`19ECh`** via native ABI (never enters `08D2h`/`0D6Bh` FF). Unpages TR-DOS, selects ROM1, restores CHANS/STRMS, FLAGS bit 7, enters `LINE-NEW`. Asserts `0x8000==0xA5`. **This 5.04 image cannot run native `012Ah`:** the load services are in a 1.6 KiB FF hole, and `012Ah` from `19ECh` re-enters catalog (`30B2h`) before LINE-NEW |

`RUN` with no filename loads the BASIC program named `boot` (Beta 128 manual).

This tree’s usual `roms/pentagon/trdos.rom` identifies as **TR-DOS Ver 5.04** with FF padding `0800h`–`0E71h` (covers `08D2h` and `0D6Bh`). That is a ROM-image limitation, not an emulator skip of working service code. Resolver prefers `roms/pentagon/trdos-5.04t.rom` / `trdos-complete.rom` (and the same names under `roms/trdos/`) when those files exist and have non-FF services — see [ROMS.md](../../../docs/ROMS.md).

| Fixture | Licence | Path | Marker |
| --- | --- | --- | --- |
| `synthetic_trdos_boot_basic` | in-repo | VG93/CPU read of BASIC `boot` | `POKE 32768,165` |

Geometry: 40-track SS, 16×256 (`disk_type = 0x19`). Directory + volume info on
track 0; `boot` body at track 1, TR-DOS start sector 0 (VG93 sector ID 1).

## Not in git

Do not commit TR-DOS ROM dumps (© Technology Research Ltd). Place a 16 KiB
image at `roms/trdos.rom`, `roms/trdos/trdos.rom`, or `roms/pentagon/trdos.rom`.
For native `08D2h`/`0D6Bh` file-load acceptance, prefer a complete 5.03 / 5.04T
dump at `roms/pentagon/trdos-5.04t.rom` or `roms/pentagon/trdos-complete.rom`
(also accepted under `roms/trdos/`). Commercial `.trd` titles stay local; do not
add them to this tree.
