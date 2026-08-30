# Timex TC2048 (Phase 1)

Timex Sinclair **TC2048** support in Spec Chum is tracked in
[#192](https://github.com/mward-sudo/spec_chum/issues/192). Phase 1 landed in
PR [#203](https://github.com/mward-sudo/spec_chum/pull/203): enough to boot the
Timex ROM and run common 48K-style software, but **not** the Timex display
hardware.

## ROM

Fetch the official distributable image with:

```bash
./scripts/fetch_roms.sh
```

Expected path: `roms/timex/tc2048.rom` (16 KiB). Grant and attribution notes:
[ROMS.md](ROMS.md) → Timex section.

The model stays disabled in pickers until that file is present (or you choose an
equivalent dump in the ROM setup dialog).

## What Phase 1 includes

| Piece | Phase 1 behaviour |
| --- | --- |
| CPU / RAM / contention | Same 48K-class core as Spectrum 48K |
| Display | Standard Sinclair **256×192** ULA path |
| ROM | Timex TC2048 firmware (`tc2048.rom`) |
| SCLD ports `0xFF` / `0xF4` | **Latched** — reads return last written values |
| SCLD video | **Not implemented** — no Timex ULA / hi-res framebuffer |

Phase 1 is deliberately **Timex ROM + SCLD port latches**, not SCLD video. The
ports exist so Timex BASIC and setup code can configure modes without faulting;
the emulator still draws through the ordinary 48K ULA.

## What works

- Boot to Timex copyright line and BASIC prompt
- Standard **256×192** text and attribute modes (looks like a 48K Spectrum)
- 48K-class session hardware where compat allows: Multiface 1
  ([MULTIFACE.md](MULTIFACE.md)), DivMMC, Interface 1, Beta Disk / TR-DOS
- Tape load, flash-load, and Type LOAD paths (same as 48K)

## Known limitations

Software that stays on the normal Spectrum screen behaves as expected. Anything
that switches the Timex into **hi-res or extended display modes** will look
wrong:

- **512×192 hi-res** — garbled or incorrectly laid out (SCLD video deferred)
- **Extended / dual-screen modes** — broken visually
- Border / paging side effects of those modes — not modelled

Do not expect Timex-specific demos, art packages, or games that depend on
512×192 to render correctly until SCLD video lands.

## Phase 2 ([#192](https://github.com/mward-sudo/spec_chum/issues/192))

Still open on the same issue:

- **TS2068 / TC2068** machine types (dual ROM banks already fetched as
  `tc2068-0.rom` / `tc2068-1.rom` — not wired yet)
- **Horizontal MMU** paging and extended memory banking
- Full **SCLD video** (512×192, extended modes, dual display)
- Snapshot / header parity if Fuse Timex formats need it

## Selecting the model

| Host | How |
| --- | --- |
| egui | **Machine → Built-in models → Timex TC2048** |
| macOS | **Machine → Built-in models → Timex TC2048** |
| Headless | `spec-chum-debug --model tc2048 …` |
| Custom profile | Base model **Timex TC2048** in a saved configuration (#187) |

Custom profiles may override the main ROM; built-in selection always uses the
fetched default unless you point the ROM setup dialog at another file.
