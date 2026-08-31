# Timex TC2048 / TS2068

Timex Sinclair support in Spec Chum is tracked in
[#192](https://github.com/mward-sudo/spec_chum/issues/192).

| Phase | Model | Status |
| --- | --- | --- |
| **1** | **TC2048** | Shipped ([#203](https://github.com/mward-sudo/spec_chum/pull/203)): 48K-class + SCLD port latches |
| **2a** | **TS2068 / TC2068** | Smoke-boot: home + EX-ROM, horizontal MMU, Timex AY ports |
| **2a+** | (same) | Warajevo **`.dck` dock** — HOME Spectrum cart smoke (PROG/`$0556`) + optional Death Chase EAR |
| **2b** | (same) | **Partial:** alt display file + hi-colour (8×1) at 256×192 — shipped this slice; **512×192 hi-res still deferred** |

Portuguese **TC2068** and US **TS2068** share the same ROM set and memory model here;
the picker label is **Timex TS2068**.

## ROM

Fetch the distributable images with:

```bash
./scripts/fetch_roms.sh
```

| Model | Paths |
| --- | --- |
| TC2048 | `roms/timex/tc2048.rom` (16 KiB) |
| TS2068 | `roms/timex/tc2068-0.rom` (16 KiB home) + `roms/timex/tc2068-1.rom` (8 KiB EX-ROM) |

Grant and attribution notes: [ROMS.md](ROMS.md) → Timex section.

Models stay disabled in pickers until the required file(s) are present (or you choose
equivalent dumps in the ROM setup dialog).

## What works today

| Piece | Behaviour |
| --- | --- |
| CPU / RAM / contention | 48K-class core |
| Display | Sinclair **256×192** + Timex **alt file** / **hi-colour (8×1)** via port `0xFF` |
| TC2048 ROM | Timex TC2048 firmware |
| TS2068 ROMs | Home BASIC + EX-ROM bank 0′ |
| SCLD ports `0xFF` / `0xF4` | Latched (both models); screen modes 0–3 drawn |
| Horizontal MMU (TS2068) | HSR bits page EX-ROM or DOCK over 8K chunks; EX-ROM mirrored per Fuse |
| AY (TS2068) | Ports `0xF5` / `0xF6` (Timex wiring); stereo pan selectable |
| Timex joysticks on AY R14 | Modelled (Fuse bit layout); host Kempston remapped to both sticks |
| Dock cartridges | Warajevo `.dck` insert/eject (TS2068); empty dock reads `0xFF` |
| SCLD hi-res video | **Not implemented** — modes 4–7 (512×192) still fall back to standard layout |

## Dock cartridges (`.dck`)

Fuse/Warajevo `.dck` images are supported on **Timex TS2068** only (insert soft-resets,
same idea as Fuse).

| Host | How |
| --- | --- |
| egui | **Hardware → Insert Timex Dock DCK…** / **Eject Timex Dock** |
| macOS | **Hardware → Insert Timex Dock DCK…** / **Eject Timex Dock** |
| FFI | `sc_insert_dck` / `sc_eject_dck` / `sc_has_timex_dock` |

Bank IDs (first header byte):

| ID | Bank | Effect |
| --- | --- | --- |
| `0` | DOCK | Visible when port `0xFF` bit 7 is clear and HSR (`0xF4`) pages the chunk |
| `254` | EX-ROM | Replaces / extends the 8 KiB EX-ROM image for paged chunks |
| `255` | HOME | Overlays home ROM/RAM chunks (stock Timex ROM kept underneath) |

Chunk access bytes `0`/`1`/`2`/`3` = absent / empty RAM / ROM / RAM-with-image (8 KiB
pages follow in file order). Multi-bank files are accepted; later banks override.

### Spectrum ROM cartridge recipes

Build a 16 KiB Spectrum ROM into a `.dck` with a 9-byte header + raw ROM:

| Goal | Header (decimal) | Notes |
| --- | --- | --- |
| DOCK Spectrum ROM | `0,2,2,0,0,0,0,0,0` | Page with `OUT 244,3` (chunks 0+1) |
| HOME Spectrum replace | `255,2,2,0,0,0,0,0,0` | Boots as Spectrum-like home (Timex → Spectrum cart) |

Helper (writes under `roms/timex/`, gitignored — never commit ROM/cart bytes):

```bash
./scripts/make_spectrum_dck.sh home   # → roms/timex/spectrum-home.dck
./scripts/make_spectrum_dck.sh dock   # → roms/timex/spectrum-dock.dck
```

Or by hand (HOME replace from fetched `roms/spec48.rom`):

```bash
{ printf '\xff\x02\x02\x00\x00\x00\x00\x00\x00'; head -c 16384 roms/spec48.rom; } > spectrum-home.dck
```

With a HOME Spectrum cart inserted, Timex `PROG` / absolute `USR` titles that expect
Spectrum ROM layout (e.g. **3D Deathchase**) can run on TS2068 the same way a real
Spectrum ROM cartridge would. Stock Timex without a cart still needs **TC2048** / **48K**
for those titles.

Automated coverage (ROMs present; Death Chase TZX optional):

| Test | Gate |
| --- | --- |
| `ts2068_home_spectrum_dck_boots_spectrum_prog` | fetched Timex + `spec48.rom` |
| `deathchase_ear_loads_on_ts2068_with_home_spectrum_dck` | + `SPEC_CHUM_DEATHCHASE_TZX` |

## Known limitations

Software that stays on the normal Spectrum screen, **alt display file** (`OUT 255,1`),
or **hi-colour** (`OUT 255,2` / `3`) behaves as expected at 256×192. **512×192 hi-res**
modes (`OUT 255,4`–`7`) still look wrong:

- **512×192 hi-res** — still drawn as standard 256×192 (SCLD hi-res deferred)
- Border colour override from hi-res ink/paper bits — not modelled
- Dedicated Timex joystick input modes (separate from Kempston) — not yet
- LROS/AROS autostart semantics beyond memory mapping — not specially modelled
  (cartridge bytes are mapped; Timex ROM still decides what to jump to)

### Spectrum tape software on TS2068

Timex BASIC `LOAD ""` over EAR works for ordinary PROGRAM/CODE that does not depend on
Spectrum ROM addresses (e.g. fixture `print_ok.tap`).

Two Spectrum habits still break on a **stock** TS2068 (same as real hardware without a
Spectrum ROM cartridge — only ~7% of commercial Spectrum titles run):

1. **`CALL $0556` (LD-BYTES)** — Timex home ROM has different code there; the relocated
   loader is at `$00FC` in EX-ROM. Spec Chum redirects **RAM** callers that land on
   `$0556` without the Spectrum prologue to that Timex entry (pages EX-ROM chunk 0').
   A HOME Spectrum `.dck` restores the real Spectrum entry at `$0556`.
2. **Absolute `USR` / PROG layout** — Spectrum `PROG` is `$5CCB` (23755); Timex `PROG`
   is `$6856` (26710). Titles such as **3D Deathchase** poke/run machine code at fixed
   Spectrum addresses and will not start on stock Timex home. Use **Timex TC2048**,
   **48K**, or a **HOME Spectrum `.dck`** on TS2068.

Do not expect Timex demos or art packages that depend on **512×192** to render
correctly until SCLD hi-res lands.

## Selecting the model

| Host | How |
| --- | --- |
| egui | **Machine → Built-in models → Timex TC2048** / **Timex TS2068** |
| macOS | **Machine → Built-in models → …** |
| Headless | `spec-chum-debug --model tc2048 …` or `--model ts2068 …` |
| Custom profile | Base model Timex TC2048 / TS2068 in a saved configuration (#187) |

Custom profiles may override ROM slots; built-in selection uses the fetched defaults
unless you point the ROM setup dialog at other files.
