# ZX Spectrum Next DMA

The Next bus implements the common single-channel zxnDMA transfer subset on
partially decoded port `$xx6B`. Port `$xx0B` remains unimplemented and is not
aliased to zxnDMA. The DMA register protocol follows the Next's variable-length
WR0–WR6 write groups, with memory or I/O endpoints, transfer direction,
increment/decrement/fixed addresses, block length, LOAD, CONTINUE, ENABLE,
DISABLE, reset, and read-mask status sequences.

Transfers use the current CPU-visible MMU mapping for memory. I/O endpoints
use the Next bus's existing port decode, including DAC ports. A zero block
length transfers zero bytes. `$C3`, `$CF`, `$D3`, `$87`, `$83`, `$BF`, `$A7`,
`$8B`, `$BB`, and the port timing reset commands are handled. Unsupported DMA
commands and interrupt/ready signaling are ignored. WR5's auto-restart bit is
accepted as part of the register byte but not implemented; transfers stop at
the end of the programmed block.

Continuous transfers run synchronously when ENABLE is written and add the
configured per-byte A/B cycle lengths (or fixed prescaler period) to CPU wait
time. Burst transfers with a nonzero prescaler schedule one byte at each
prescaler interval while the CPU continues. The nominal period is four 3.5 MHz
T-states per prescaler count, bounded below by the configured A/B bus cycles.
This gives deterministic 16-bit-machine timing for audio use, but does not
follow NextReg `$11` clock changes.

The core does not arbitrate DMA at individual memory cycles. Burst events are
serviced at CPU instruction boundaries, so DMA and CPU accesses within one Z80
instruction are not interleaved. ULA contention, CPU turbo rates, DMA interrupt
generation, Z80-DMA compatibility on `$xx0B`, and board-level ready signaling
are outside this model.
