# Spectrum Next timing comparisons

Issue [#551](https://github.com/mward-sudo/spec_chum/issues/551) tracks differential checks against MAME and ZEsarUX. This page records only cases that have actually run; a matching result for one probe does not establish general emulator parity.

## Run the reference probe

The runner builds a temporary, 17 KiB NEX guest and runs it in both references. It uses a local ZEsarUX remote-command connection and a headless MAME debugger. It requires Python 3, a MAME executable, the upstream `bootrom.vhd` source file, and a ZEsarUX executable (non-macOS) or `.app` bundle (macOS). It extracts the ROM into a temporary MAME ROM path after checking its size and SHA-1.

```sh
python3 scripts/next_reference_probe.py \
  --bootrom-vhdl /path/to/bootrom.vhd \
  --zesarux /Applications/ZEsarUX.app
```

The MAME video backend uses SDL's dummy driver. ZEsarUX starts with video and audio output disabled, without reading or writing its configuration, and communicates only through `127.0.0.1:10000`. On macOS the runner requires the `.app` bundle and asks LaunchServices to start a new hidden instance; it never launches the bundle executable directly. It refuses to connect if that port is already in use and exits its own instance when the run ends.

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

The timing and marker agree, while CTC status differs. The official [CTC description](https://wiki.specnext.dev/CTC) documents the channel timer, and [NextReg `$C9`](https://wiki.specnext.dev/Interrupt_Status_1_Register) records whether a CTC interrupt occurred or is pending. Spec Chum's `ctc_hardware_im2_interrupt_runs_guest_handler_and_reti_releases_service` test expects the channel status bit after overflow, consistent with MAME's observation. This comparison does not establish which reference matches physical hardware; the CTC difference remains unresolved and #551 stays open.

Fixture SHA-256: `e5b123c972a3bcb71d312c649a3b3bb985cfb3aef7a3c1b9f78e2cc940edf126`.

This validates the guest fixture, the two capture paths, and this instruction sequence only. It does not compare frame timing, interrupt entry timing, video/register changes, audio, or DMA. Keep #551 open until its selected timing-sensitive cases have deterministic reference results and actionable differences have follow-up issues.

## Local baseline

The Next machine suite remains the local oracle; it does not replace reference observations:

```sh
CARGO_TARGET_DIR=/tmp/spec-chum-next-timing-target cargo test -p machine next:: --release
```
