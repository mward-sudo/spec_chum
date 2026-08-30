# Timex TC2048 / TS2068

Timex Sinclair support in Spec Chum is tracked in
[#192](https://github.com/mward-sudo/spec_chum/issues/192).

| Phase | Model | Status |
| --- | --- | --- |
| **1** | **TC2048** | Shipped ([#203](https://github.com/mward-sudo/spec_chum/pull/203)): 48K-class + SCLD port latches |
| **2a** | **TS2068 / TC2068** | Smoke-boot: home + EX-ROM, horizontal MMU, Timex AY ports |
| **2b** | (same) | Full **SCLD video** (512×192, extended / dual-screen) — deferred |

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
| Display | Standard Sinclair **256×192** ULA path |
| TC2048 ROM | Timex TC2048 firmware |
| TS2068 ROMs | Home BASIC + EX-ROM bank 0′ |
| SCLD ports `0xFF` / `0xF4` | Latched (both models) |
| Horizontal MMU (TS2068) | HSR bits page EX-ROM (or empty DOCK) over 8K chunks; EX-ROM mirrored per Fuse |
| AY (TS2068) | Ports `0xF5` / `0xF6` (Timex wiring); stereo pan selectable |
| Timex joysticks on AY R14 | Not modelled yet |
| Dock cartridges | Empty (reads `0xFF`); no `.dck` loader |
| SCLD video | **Not implemented** — no Timex ULA / hi-res framebuffer |

## Known limitations

Software that stays on the normal Spectrum screen behaves as expected. Anything that
switches into **hi-res or extended display modes** will look wrong:

- **512×192 hi-res** — garbled or incorrectly laid out (SCLD video deferred)
- **Extended / dual-screen modes** — broken visually
- Border / paging side effects of those modes — not modelled
- Dock software and Timex joystick ports — not yet

Do not expect Timex-specific demos, art packages, or games that depend on 512×192 to
render correctly until SCLD video lands.

## Selecting the model

| Host | How |
| --- | --- |
| egui | **Machine → Built-in models → Timex TC2048** / **Timex TS2068** |
| macOS | **Machine → Built-in models → …** |
| Headless | `spec-chum-debug --model tc2048 …` or `--model ts2068 …` |
| Custom profile | Base model Timex TC2048 / TS2068 in a saved configuration (#187) |

Custom profiles may override ROM slots; built-in selection uses the fetched defaults
unless you point the ROM setup dialog at other files.
