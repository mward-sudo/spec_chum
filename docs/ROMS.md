# System ROMs — fetch, copyright, attribution

Spec Chum’s MIT `LICENSE` covers **our code only**. Spectrum (and related)
firmware images are separate works.

## Official Sinclair / Amstrad Spectrum ROMs

Cliff Lawson (Amstrad plc, 1999-08-31) stated that Amstrad are happy for
emulator writers to include images of their copyrighted Spectrum ROM code
**as long as the (c)opyright messages inside the images are not altered**, and
asked that the program/manual note:

> Amstrad have kindly given their permission for the redistribution of their
> copyrighted material but retain that copyright.

That grant covers the classic Spectrum family ROMs Spec Chum fetches today
(16K / 48K / 128K / grey +2 / +2A / +3). Amstrad (and Sinclair as original
author) retain copyright. Do **not** strip or patch the copyright strings
inside `.rom` files.

### What this project does today

- System ROMs are **not** committed to git and are **not** attached to GitHub
  Releases ([RELEASE.md](RELEASE.md)).
- Fetch images with `./scripts/fetch_roms.sh` (pinned sparse checkouts — see
  [Fetch inventory](#fetch-inventory) below).
- Official UK primary paths used by the emulator today:
  `roms/spec48.rom`, `roms/128/spec128uk.rom`, `roms/plus2/plus2uk.rom`,
  `roms/plus2a/plus2a.rom`, `roms/plus3/plus3.rom`.

If we ever ship ROM bytes with a build, the Lawson notice above remains
required, and in-image copyright messages must stay intact.

## Fetch inventory

`./scripts/fetch_roms.sh` installs **40** distributable `.rom` files into
`roms/` (refs recorded in `roms/.zx-roms-ref` and `roms/.fuse-roms-ref`).
Tracking: [#190](https://github.com/mward-sudo/spec_chum/issues/190) /
licence matrix on [#188](https://github.com/mward-sudo/spec_chum/issues/188#issuecomment-5465132775).

### Amstrad / Sinclair — [spectrumforeveryone/zx-roms](https://github.com/spectrumforeveryone/zx-roms)

| Destination | Grant | Notes |
| --- | --- | --- |
| `roms/spec48.rom` | Lawson 1999 | UK 48K (also used for 16K model) |
| `roms/alternate/*.rom` | Lawson 1999 | Arabic / Beckman / prototype 48K variants |
| `roms/128/*.rom` | Lawson 1999 | UK + Spanish + Derby monitor 128K |
| `roms/plus2/*.rom` | Lawson 1999 | UK + Spanish + French grey +2 |
| `roms/plus2a/*.rom` | Lawson 1999 | UK + Spanish +2A |
| `roms/plus3/*.rom` | Lawson 1999 | UK + Spanish +3 |

Sparse checkout **excludes** `peripherals/Interface1` and `zx80-81` (see
[user-provided only](#user-provided-only--not-auto-fetched) below).

### Fuse 16 KiB bank splits — [fuse-emulator/fuse `roms/`](https://github.com/fuse-emulator/fuse/tree/master/roms)

| Destination | Grant | Notes |
| --- | --- | --- |
| `roms/fuse-16k/*.rom` | Lawson 1999 | Same official UK machines as 16 KiB banks (`48.rom`, `128-*.rom`, …) |

### Timex — Fuse `roms/` ([`README.copyright`](https://github.com/fuse-emulator/fuse/blob/master/roms/README.copyright))

| Destination | Grant | Notes |
| --- | --- | --- |
| `roms/timex/tc2048.rom` | Lawson 1999 / Amstrad copyright (Fuse README.copyright) | TC2048 Phase 1 ([#192](https://github.com/mward-sudo/spec_chum/issues/192)); see [TIMEX.md](TIMEX.md) |
| `roms/timex/tc2068-0.rom`, `tc2068-1.rom` | Lawson 1999 base + Timex modifications PD per Fuse copyright file | TC2068 Phase 2 ([#192](https://github.com/mward-sudo/spec_chum/issues/192)); fetched early, not wired yet |

### OpenSE BASIC — Fuse `se-*.rom` (GPL-2+)

| Destination | Grant | Notes |
| --- | --- | --- |
| `roms/opense/se-0.rom`, `se-1.rom` | GPL-2+ | Optional free 48K alternate — not for Amstrad accuracy testing |

### +3e — Fuse `plus3e-*.rom`

| Destination | Grant | Notes |
| --- | --- | --- |
| `roms/plus3e/plus3e-0.rom` … `plus3e-3.rom` | Amstrad modify/distribute + Garry Lancaster free distribution; Fuse ships | Optional enhanced +3 ([#194](https://github.com/mward-sudo/spec_chum/issues/194)) |

### Datel +D / DISCiPLE — Fuse `disciple.rom`, `plusd.rom`

| Destination | Grant | Notes |
| --- | --- | --- |
| `roms/peripherals/datel/disciple.rom` | [Datel grant](https://www.shadowmagic.org.uk/spectrum/datel.html) (Philip Kendall correspondence) | When +D/DISCiPLE emulation lands |
| `roms/peripherals/datel/plusd.rom` | Same | |

### SpeccyBoot — Fuse `speccyboot-1.4.rom` (MIT)

| Destination | Grant | Notes |
| --- | --- | --- |
| `roms/peripherals/speccyboot/speccyboot-1.4.rom` | MIT (Patrick Persson) | `LICENSE` in same directory; only if SpeccyBoot is emulated |

## User-provided only — not auto-fetched

Supply your own images (or wait for a new written grant). **Do not** extend
`fetch_roms.sh` to these without updating this doc and issue [#190](https://github.com/mward-sudo/spec_chum/issues/190).

| Firmware | Why not fetched |
| --- | --- |
| Interface 1 / Microdrive | Not Amstrad’s grant; Lawson excludes IF1; zx-roms `info.txt` conflicts |
| Multiface 1 / 128 / +3 | © Romantic Robot; commercial emulator licences are exclusive |
| TR-DOS / Beta Disk | © Technology Research; Fuse dropped redistribution |
| ESXDOS / DivMMC EEPROM | Public download ≠ redistribution grant ([#138](https://github.com/mward-sudo/spec_chum/issues/138)) |
| Pentagon / Scorpion | Clone firmware; user dumps ([#193](https://github.com/mward-sudo/spec_chum/issues/193)) |
| Opus Discovery | No clear grant (Fuse dropped) |
| TK90X / Didaktik / Inves clones | No public grant found ([#195](https://github.com/mward-sudo/spec_chum/issues/195)) |
| ZX80 / ZX81 | Nine Tiles rights; zx-roms “kind permission” unverified ([#188](https://github.com/mward-sudo/spec_chum/issues/188)) |
| Spectrum Next firmware | Separate Next licence ([#191](https://github.com/mward-sudo/spec_chum/issues/191)) |

### Pentagon 128 — user-provided paths (#188 Phase B)

Place your own dumps (never committed; not fetched by `fetch_roms.sh`):

| File | Size | Notes |
| --- | --- | --- |
| `roms/pentagon/pentagon.rom` | 32 KiB | Main Pentagon firmware (also accepts `128p.rom`) |
| `roms/pentagon/trdos.rom` | 16 KiB | TR-DOS for Beta Disk — both files required before the model enables |

Timing: 71680 T-states/frame (320×224), no Sinclair memory contention. TR-DOS boot depth still tracked in [#140](https://github.com/mward-sudo/spec_chum/issues/140).

### Timex TC2048 — fetched path (#192 Phase 1)

After `./scripts/fetch_roms.sh`:

| File | Size | Notes |
| --- | --- | --- |
| `roms/timex/tc2048.rom` | 16 KiB | Required for **Timex TC2048** in Machine pickers |

Phase 1 = Timex ROM + SCLD port latches (`0xFF` / `0xF4`), **not** SCLD video.
Standard **256×192** works; **512×192** hi-res and extended modes are visually
broken until Phase 2. Full scope, limits, and Phase 2 (TS2068 / TC2068, horizontal
MMU): [TIMEX.md](TIMEX.md).

Peripheral attach UX: Multiface ([MULTIFACE.md](MULTIFACE.md)), Interface 1,
Beta, DivMMC — see GitHub issues #137–#140 / #169.

## Manual verify

After `./scripts/fetch_roms.sh`:

```bash
# Managed fetch set (ignores user-provided dumps you may add under roms/)
find roms/{alternate,128,plus2,plus2a,plus3,fuse-16k,timex,opense,plus3e,peripherals} \
  -name '*.rom' 2>/dev/null | wc -l   # expect 39, plus roms/spec48.rom → 40 total
cat roms/.zx-roms-ref roms/.fuse-roms-ref
```

User-provided firmware (Multiface, TR-DOS, etc.) may also live under `roms/` but is
**not** part of the managed fetch count.

Compare bytes against a known-good tree if refreshing upstream pins (see issue
[#190](https://github.com/mward-sudo/spec_chum/issues/190) acceptance criteria).

## References

- [DeZog `amstrad-rom-permissions.txt`](https://github.com/maziac/DeZog/blob/main/documentation/amstrad-rom-permissions.txt) (full Lawson post)
- [Fuse `roms/README.copyright`](https://github.com/fuse-emulator/fuse/blob/master/roms/README.copyright)
- [Datel +D/DISCiPLE permission](https://www.shadowmagic.org.uk/spectrum/datel.html)
- Spectaculator / Debian `spectrum-roms` legal notes (same Lawson wording)
- Licence matrix comment on [#188](https://github.com/mward-sudo/spec_chum/issues/188#issuecomment-5465132775)
