# Spectrum Next CTC subset

The Next bus implements four Zilog CTC channels on `$183B`, `$193B`, `$1A3B`,
and `$1B3B`. The high-byte low three bits select channels 0–3; ports `$1C3B`
through `$1F3B` are currently unimplemented because the current Next core
exposes only four channels.

The implementation supports timer and cascaded counter operation, control
words, time-constant-follow writes (including zero as 256), the 16/256 timer
prescaler, trigger-start behavior and trigger-edge changes, reset, down-counter
readback, one-clock zero-count outputs, channel 0–3 cascading (including the
channel 3 to channel 0 wrap), and CTC IRQs.
Channel 0 has highest CTC priority. The Z80 `RETI` instruction releases the
active CTC daisy-chain service state.

NextReg `$C5` enables the implemented channel interrupt sources in bits 0–3.
NextReg `$C9` reports CTC interrupt history or pending state in bits 0–3; write
one to clear a history bit after its pending request has been acknowledged.
NextReg `$C5` writes the channels' effective interrupt-enable bits (the same
control D7 state set by a CTC control word). A disabled source remains pollable
through counter readback and `$C9` status.

CTC time advances on the FPGA's 28 MHz master clock (eight CTC clocks per
3.5 MHz video T-state). NextReg `$07` changes CPU T-state length at the next
instruction boundary; the CTC and video frame periods remain on the master
clock. The machine test programs all four CPU speeds and checks the same CTC
count after equal elapsed master time.
In pulse mode, the CTC interrupt acknowledge uses the ordinary `$FF` data-bus
value. In Hardware IM2 mode (NextReg `$C0` bit 0), the four CTC channels occupy
source slots 3–6 and the vector is `($C0 & $E0) | ((3 + channel) << 1)`.
`$C0` supports only vector-base and mode bits for this slice; other interrupt
sources, external physical CLK/TRG inputs, and channels 4–7 are not implemented.

Reference: [SpecNext CTC documentation](https://wiki.specnext.dev/CTC) and the
[Zilog Z80 CPU Peripherals User Manual](https://www.zilog.com/docs/z80/um0081.pdf).
