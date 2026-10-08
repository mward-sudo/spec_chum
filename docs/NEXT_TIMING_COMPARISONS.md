# Spectrum Next timing comparisons

Issue [#551](https://github.com/mward-sudo/spec_chum/issues/551) tracks differential checks against MAME and ZEsarUX. This page records only cases that have actually run; a matching result for one probe does not establish general emulator parity.

## Run the reference probe

The runner builds temporary, 16,896-byte NEX guests (16.5 KiB each) and runs them in both references. It uses a local ZEsarUX remote-command connection and a headless MAME debugger. It requires Python 3, a MAME executable, the upstream `bootrom.vhd` source file, and a ZEsarUX executable (non-macOS) or `.app` bundle (macOS). It extracts the ROM into a temporary MAME ROM path after checking its size and SHA-1.

```sh
python3 scripts/next_reference_probe.py \
  --bootrom-vhdl /path/to/bootrom.vhd \
  --zesarux /Applications/ZEsarUX.app
```

The MAME video backend uses SDL's dummy driver. ZEsarUX starts with video and audio output disabled, without reading or writing its configuration, and communicates only through `127.0.0.1:10000`. On macOS the runner copies the `.app` bundle to its temporary directory with `ditto`, adds the owner search bit to directories in that copy, and asks LaunchServices to start a new hidden instance with `open -g -j -W -a`. It never launches the bundle executable directly and never changes the supplied app bundle. It refuses to connect if that port is already in use and exits its own instance when the run ends. Run the probe outside a restrictive agent sandbox: sandboxed LaunchServices can reject even a correctly staged bundle with `kLSNoExecutableErr` (`-10827`). This sandbox failure is separate from a ZEsarUX emulator crash.

For the MAME 0.289 `v30100` BIOS, the expected SHA-1 is `8b3c2a301f486904d1c74929b94845a7731bf230`. The matching 8 KiB image is extracted from the public [ZX Spectrum Next FPGA core source](https://gitlab.com/SpectrumNext/ZX_Spectrum_Next_FPGA) (`bootrom.vhd`, SHA-256 `beba481a4728faa027ca3f559d4b2c87a6042ee821d8dfd025aacd8b72b49188`). The extracted image exists only in the runner's temporary directory as `tbblue/boot-30100.bin`. The project's official System/Next 24.11 `boot-30204.bin` is a different image and must not be renamed to satisfy MAME's checksum.

## Case: base instruction cycles and CTC status

The generated NEX loads this program at `$C000` in bank 2:

```text
LD A,$2A          ; 7 T-states
LD ($C100),A      ; 13 T-states
LD BC,$243B       ; select NextReg port
LD A,$C5          ; select CTC interrupt enable
OUT (C),A
LD A,1            ; enable CTC channel 0
OUT (C),A
LD BC,$183B       ; select CTC channel 0
LD A,$85          ; enable timer interrupt; expect a time constant
OUT (C),A
LD A,1            ; start timer at its minimum interval
OUT (C),A
LD BC,$243B       ; select NextReg port
LD A,$C9          ; select CTC status NextReg
OUT (C),A
LD BC,$253B       ; NextReg access port
IN A,(C)          ; read CTC status
LD ($C101),A      ; save status
loop: JP loop
```

The runner records the base timing from `$C000` to `$C005`, then measures through `$C02A` and checks the marker and CTC status bytes. Its deterministic result is:

| Reference | Version | Base T-states `$C000` → `$C005` | Total T-states `$C000` → `$C02A` | `$C100` marker | `$C101` CTC status |
| --- | --- | ---: | ---: | ---: | ---: |
| MAME | 0.289 | 20 | 180 | `$2A` | `$01` |
| ZEsarUX | 13.0 | 20 | 180 | `$2A` | `$00` |

The timing and marker agree, while CTC status differs. The official [CTC description](https://wiki.specnext.dev/CTC) documents the channel timer, and [NextReg `$C9`](https://wiki.specnext.dev/Interrupt_Status_1_Register) records whether a CTC interrupt occurred or is pending. Spec Chum's `ctc_hardware_im2_interrupt_runs_guest_handler_and_reti_releases_service` test expects the channel status bit after overflow, consistent with MAME's observation.

Source inspection classifies this as a reference-coverage difference. [MAME 0.289's Next driver](https://github.com/mamedev/mame/blob/mame0289/src/mame/sinclair/next/specnext.cpp) maps `$C5` to its CTC interrupt control and `$C9` to CTC interrupt status; [its CTC device](https://github.com/mamedev/mame/blob/mame0289/src/mame/sinclair/next/specnext_ctc.cpp) implements four channels.

In the [ZEsarUX 13.0 source tree](https://github.com/chernandezba/zesarux/tree/ZEsarUX-13.0/src/machines), `tbblue.c` mentions CTC channels in the interrupt-priority description, but we found no CTC device or port handler in the tagged source. Its `$00` result therefore does not independently test the CTC behavior. This is not evidence that MAME matches physical hardware or that Spec Chum is wrong. Treat the case as a ZEsarUX 13.0 capability gap; keep #551 open for comparisons with independent coverage, including interrupt boundaries and video/register changes.

## Case: Z80N `MUL D,E` timing and result

The second fixture sets `DE=$1317`, executes the Next-specific `ED 30` (`MUL D,E`), saves the result bytes to `$C101`/`$C102`, and pads the same `$C000`–`$C02A` probe window with known instructions. The [official Z80 instruction table](https://wiki.specnext.dev/Extended_Z80_instruction_set) specifies this opcode as 8 T-states and computes `DE := D*E`; `$13 * $17 = $01B5`. The expected aggregate is 20 T-states from `$C000` to the `$C005` baseline, then 134 T-states through the stop boundary (including the 8-T-state multiply), for 154 total. The fixture returned the same readings in three consecutive runs against each reference.

| Reference | Version | Base T-states `$C000` → `$C005` | Total T-states `$C000` → `$C02A` | `$C100` marker | `$C101:$C102` result |
| --- | --- | ---: | ---: | ---: | ---: |
| MAME | 0.289 | 20 | 154 | `$2A` | `$01:$B5` |
| ZEsarUX | 13.0 | 20 | 154 | `$2A` | `$01:$B5` |

The generated NEX is 16,896 bytes; SHA-256: `bfb95ad94e4dd4a191e4fb446ec02b4dd72c51ed3971878b13c0b861b88d57c7`. This is independent of the CTC path and establishes matching execution of this instruction sequence in these two emulator versions. It is not a physical-hardware measurement or a broad comparison of video, frame timing, or interrupt entry.

## Case: active video line advances

The third fixture writes `0` to NextReg `$07` to select 3.5 MHz CPU speed, then polls the active video line MSB `$1E` and LSB `$1F` until the full line number is `$020`. Checking the MSB avoids also accepting line `$120`, which has the same low byte. It saves the line to `$C100`, waits about 369 CPU T-states, reads the line again, and saves it to `$C101`. The polling loop establishes a known raster phase after the NEX loader, so the comparison does not assume identical loader timing. The stop PC is `$C03B`.

MAME's debugger runs to the stop PC. ZEsarUX uses ZRCP `run no-stop-on-data` with a PC breakpoint, which executes its normal remote core loop. The [MAME 0.289 Next driver](https://github.com/mamedev/mame/blob/mame0289/src/mame/sinclair/next/specnext.cpp#L1625-L1630) and [ZEsarUX 13.0 Next implementation](https://github.com/chernandezba/zesarux/blob/ZEsarUX-13.0/src/machines/tbblue.c#L5583-L5598) both serve `$1F` from the raster line. The three direct probe runs and the integrated runner produced the same values:

| Reference | Version | `$C100` first line | `$C101` second line |
| --- | --- | ---: | ---: |
| MAME | 0.289 | `$20` | `$21` |
| ZEsarUX | 13.0 | `$20` | `$21` |

Fixture SHA-256: `c9bb794544b9330596651d96eaa39fca5c6608010712b06344d99e1fbab893f4`. This confirms one line increment in each reference for this guest sequence. It does not establish framebuffer output, a complete frame duration, or physical-hardware timing.

CTC fixture SHA-256: `e5b123c972a3bcb71d312c649a3b3bb985cfb3aef7a3c1b9f78e2cc940edf126`.

These results validate the guest fixtures, capture paths, and selected instruction sequences only. They do not compare complete frame timing, interrupt entry timing, framebuffer output, audio, or DMA. Keep #551 open until its selected timing-sensitive cases have deterministic reference results and actionable differences have follow-up issues.

## Local baseline

The Next machine suite remains the local oracle; it does not replace reference observations:

```sh
CARGO_TARGET_DIR=/tmp/spec-chum-next-timing-target cargo test -p machine next:: --release
```
