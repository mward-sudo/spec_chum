# Spectrum Next Copper support

The Next bus implements the Copper's 2 KiB byte-addressed instruction memory
through NextRegs `$60`–`$62`. `$60` writes and advances the byte address;
`$61` holds its low byte and `$62` combines the high address bits with the
STOP/START mode. The 1,024 instruction list supports MOVE, WAIT, NOOP, and
HALT. MOVE uses the normal NextReg write path, and writes to registers `$80`
and above cannot be encoded.

The machine advances the Copper alongside CPU T-states. WAIT decodes the
documented 9-bit line and 6-bit horizontal coordinate; horizontal steps are
mapped to four CPU T-states using the base two-pixels-per-T-state raster. A
MOVE takes two 28 MHz Copper clocks and NOOP/WAIT one. Register effects become
visible in machine state at their scheduled beam position. The renderer keeps
the latest Copper display state for each affected raster line, so pixels before
and after a MOVE can differ in the same rendered frame. Horizontal effects are
currently quantized to whole scanlines; writes sharing a line take effect at
that line's start in the rendered output.

The renderer preserves raster changes to NextReg `$14`, the global transparency
colour. Copper cannot write the ULA border port, so it cannot change the border
colour; CPU port writes remain a separate path.

This is a bounded functional implementation, not a complete FPGA timing model.
Core 3's precise subpixel ordering and interactions among simultaneous CPU,
DMA, and Copper accesses are not modeled. WAIT lines must be within the selected
display timing's frame (up to line 319 for Pentagon timing); out-of-frame waits
stall the list. The horizontal mapping and supported NextReg side effects should
be compared with hardware before making cycle-accuracy claims.

References: [Copper instruction and control registers](https://wiki.specnext.dev/Copper)
and [official NextReg reference](https://gitlab.com/SpectrumNext/ZX_Spectrum_Next_FPGA/-/blob/master/cores/zxnext/nextreg.txt).
