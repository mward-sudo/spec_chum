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

48K + Beta enters TR-DOS from Sinclair BASIC with `RANDOMIZE USR 15616` (command
mode) or `RANDOMIZE USR 15619: REM: …` (one command, then back to BASIC). `RUN`
with no filename loads the BASIC program named `boot` (Beta 128 manual). TR-DOS
5.04 (`BETA 128`) still returns through the 48K Interface 1 trampoline when
`CHANS < 5D25h`, so a working ROM-gated `RUN` needs 128K sysvars / trampolines
and is **not** claimed here.

| Fixture | Licence | Path | Marker |
| --- | --- | --- | --- |
| `synthetic_trdos_boot_basic` | in-repo | VG93/CPU read of BASIC `boot` | `POKE 32768,165` |

Geometry: 40-track SS, 16×256 (`disk_type = 0x19`). Directory + volume info on
track 0; `boot` body at track 1, TR-DOS start sector 0 (VG93 sector ID 1).

## Not in git

Do not commit TR-DOS ROM dumps (© Technology Research Ltd). Place a 16 KiB
image at `roms/trdos.rom`, `roms/trdos/trdos.rom`, or `roms/pentagon/trdos.rom`.
Commercial `.trd` titles stay local; do not add them to this tree.
