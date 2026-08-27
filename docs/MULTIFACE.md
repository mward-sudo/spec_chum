# Multiface 1

Spec Chum emulates the common late **Multiface One** (Romantic Robot, ~pcb 2.1) on
**Spectrum 48K** only. Multiface 128 / +3 are not implemented yet ([#137](https://github.com/mward-sudo/spec_chum/issues/137)).

## ROM sourcing (user-supplied)

Multiface firmware is **not** redistributed in this repository (same policy as system
ROMs). Provide an **8 KiB** (8192-byte) `.rom` / `.bin` image yourself, for example from
a dump of your own Multiface One or from community archives that legally host dumps.

Attach via:

- **egui:** Hardware → **Attach Multiface 1 ROM…** then **Multiface NMI**
- **macOS:** Hardware → **Attach Multiface 1 ROM…** then **Multiface NMI**
  (`sc_attach_multiface` / `sc_multiface_nmi`)
- **API:** `Machine::attach_multiface(&[u8; 8192])` then `Machine::multiface_nmi()`

Wrong size is rejected. Empty/zeroed images are accepted for unit tests but will not
show a real Multiface menu.

## Behaviour modelled

| Piece | Behaviour |
| --- | --- |
| Memory | 8 KiB ROM at `0000–1FFF`, 8 KiB RAM at `2000–3FFF` when paged |
| Red button | Asserts NMI pending; Z80 NMI → `0x0066`; vector latch pages MF in |
| `IN` A7=1 (typ. `0x9F`) | Page **in** (toolkit “return”) |
| `IN` A7=0 (typ. `0x1F`) | Page **out** + Kempston joy bits D0–D4 |
| `OUT` (same decode) | Clears NMI pending only (does **not** page out) |
| Reset | Clears paging + NMI pending; **keeps** MF RAM |

Port decode matches Fuse/MAME late MF1: `(port & 0x72) == 0x12`.

## Gaps

- No Multiface 128 / Multiface 3
- No hardware enable/disable (“stealth”) switch
- No game-level soak with a real MF ROM (save to tape / return-to-game)
- MF1’s onboard Kempston shares the bus Kempston state (no separate stick)
