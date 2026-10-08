# Spectrum Next hardware inventory

This inventory compares the emulator's current Next bus/machine implementation with one pinned upstream specification snapshot. It is a coverage map, not a conformance claim.

## Source and verification

The source is the official Spectrum Next FPGA repository at commit [`dc8d76419a6968d6c93bfa5f62231a40ad8b4221`](https://gitlab.com/SpectrumNext/ZX_Spectrum_Next/-/commit/dc8d76419a6968d6c93bfa5f62231a40ad8b4221). The extracted files are pinned by SHA256 in [`docs/next/manifest.json`](next/manifest.json). The register inventory has 145 distinct documented register numbers. Repeated sprite headings `$75`–`$79` are deduplicated by register number. The peripheral-port inventory has 58 table rows; duplicate addresses remain separate because their direction, decode pattern, alias, or device meaning differs. The source's addressless +3 floating-bus row is retained with an empty `address` field.

Run the standard-library checker from the repository root:

```sh
python3 scripts/check_next_inventory.py
```

This checks complete register-ID coverage, exact ordered port-table coverage (including aliases), required fields, and allowed status values. To verify the extracted manifest against the pinned source files as well, place the source files at `SOURCE_DIR/nextreg.txt` and `SOURCE_DIR/ports.txt` and run:

```sh
python3 scripts/check_next_inventory.py --source-dir SOURCE_DIR
```

The optional source check verifies both SHA256 values and reparses the documented IDs and port entries. The checker intentionally does not fetch upstream content or require network access. After changing the pinned source version, regenerate and review the manifest and both inventories together.

## Reading the inventory

- [`docs/next/nextreg.csv`](next/nextreg.csv) has one row per documented register: title, documented access, current status, implementation path, concise documented bit fields extracted from the pinned source, current behavior/limits, exact test names where manually verified, and follow-up disposition.
- [`docs/next/ports.csv`](next/ports.csv) has one row per source port-table entry. `pattern` records the decode mask from upstream; `address` is the printed address when supplied. This preserves overlaps such as DAC/Kempston aliases and port directions.
- `implemented` means the documented behavior listed for that entry is covered by this implementation; it is not a claim of full system conformance.
- `partial` means the repository has a handler or implementation path, while documented behavior is absent or not established.
- `intentionally_unsupported` marks hardware-specific behavior deliberately outside this emulator's modeled scope.
- `unimplemented` means no corresponding handler was found in the current Next bus. Reads commonly fall through to `0xFF`; that is emulator behavior, not a claim about the physical device's returned value.
- `reserved` records a source-defined reserved register. It remains listed so future source drift cannot silently hide it.
- A test cell names only an exact test manually matched to that entry. `none verified in this inventory pass` means no test is asserted here; it is not proof that no indirect test exists.
- Follow-up cells avoid creating work solely because the hardware specification is broader than this emulator. Track a gap when a concrete compatibility requirement and acceptance check are known.

## Stateful behavior and timing

The model has state beyond a simple register-value array. These behaviors matter when validating sequences and time-dependent access:

| Area | Stateful behavior in this implementation | Evidence / limit |
| --- | --- | --- |
| Reset and boot (`$02`–`$04`) | Reset requests, DivMMC NMI request, boot-ROM removal, machine selection, timing selection, and configuration-mode SRAM mapping affect subsequent memory and instruction fetches. | `crates/bus/src/next.rs`; current reset/config tests exercise selected paths. The full reset-signal, ESP, Multiface, and I/O-trap behavior is not modeled. |
| CPU speed (`$07`) | Programmed speed is committed at the machine instruction boundary. CPU T-states convert to 8/4/2/1 master ticks. CTC advances at 28 MHz master-clock resolution; raster, audio, DMA, and Copper remain quantized to the base video-T timeline in this implementation. | `cpu_speed_register_separates_programmed_and_actual_rates_and_resets`; `cpu_turbo_runs_more_instructions_per_frame_without_speeding_up_ctc`. Sub-video timing precision for raster/audio/DMA/Copper at turbo speeds remains unimplemented and needs differential validation. |
| Peripheral controls (`$06`, `$08`–`$0A`) | AY mode/reset, Turbo Sound selection, DAC gating, SD chip select, DivMMC control, and paging lock alter later I/O and audio behavior. | `crates/bus/src/next.rs`; behavior is limited to the listed software-visible subset, not analog output or external peripherals. |
| MMU and ROM (`$50`–`$57`, `$8C`, `$8E`) | Eight 8 KiB slots map physical RAM pages; configuration and legacy 128K mapping can select ROM/RAM layouts; DivMMC fetch automapping can override visible mapping. | `crates/bus/src/next.rs`; tested mapping and selected automap sequences do not establish all hardware combinations. |
| Video registers and palettes | Palette index/data writes have 8-bit and two-byte 9-bit forms; palette auto-increment and selected palette state affect later pixels. Clip-window and sprite-attribute registers have cursor/index side effects. | `crates/bus/src/next_video.rs`, `next_sprites.rs`; implemented modes are partial. Raster-visible writes are captured by `NextMachine`; complete mid-line hardware timing is not established. |
| Copper (`$60`–`$62`) | Program/data/control state advances against raster master time; timed MOVE operations update supported video state. | `crates/bus/src/next_copper.rs`, `crates/machine/src/next.rs`; only implemented registers/features participate. |
| CTC (`$C0`, `$C5`, `$C9`, `$183B`–`$1F3B`) | Port and machine accesses advance the timer; interrupt enable/status, priority acknowledge, IM2 vectors, in-service state, and RETI affect the Z80 interrupt path. | `ctc_hardware_im2_interrupt_runs_guest_handler_and_reti_releases_service`; four channels are modeled, while the pinned table describes eight. Other CTC modes remain a gap. |
| DMA and audio timing | zxnDMA transfers can be immediate or scheduled; DAC and AY changes are timestamped and sampled on machine time. | `crates/bus/src/next_dma.rs`, `next_ctc.rs`, and `crates/machine/src/next.rs`; the legacy z80DMA interface and board-connected playback are absent. |

## Concrete known gaps

The CSVs enumerate the full documented surface. These are grouped omissions visible in the current implementation, not claims that every omitted item should be implemented:

1. **Board peripherals and external devices:** I2C, UART, ESP/Raspberry Pi GPIO and I2S, XADC, external-bus controls, Kempston mouse, Multiface, and ULA+ port behavior have no equivalent full device path. SD SPI works only when a disk image is attached; physical flash/RPi devices are not present.
2. **Legacy add-ons:** +3 FDC, z80DMA, and the full range of legacy machine-specific decode/device interactions are absent. The common Spectrum paging, keyboard, ULA, AY, and selected Next aliases are implemented as documented in `ports.csv`.
3. **Next peripheral edge cases:** the four-channel CTC subset does not implement all eight source-described channels or modes. The CPU-speed and fixed-master-clock relationship is covered by focused tests, but not yet by a full upstream timing comparison. Some interrupt sources, NMI traps, and reset signal side effects remain unmodeled.
4. **Advanced video and sprite behavior:** implemented Layer 2, tilemap, LoRes/Radastan, palette, Copper, and base sprite paths cover only the explicitly dispatched subsets. Extended sprite attributes/modes, collision behavior, advanced blending, and other undocumented-by-code modes remain incomplete.

No follow-up issue is created merely to turn this inventory into a parity project. Split a gap into a tracked issue when a concrete software compatibility case, desired behavior, and deterministic acceptance test can be stated.

## Updating the inventory

When implementation changes, update its row's status, owning source path, exact behavior limit, exact test names or explicit missing-test note, and follow-up disposition in the same change. Run the checker and inspect both CSV diffs. When changing the source pin, update `manifest.json` and every affected row from the newly pinned source; run the optional source verification before review.
