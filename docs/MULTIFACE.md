# Multiface 1 / Multiface 128

Spec Chum emulates:

| Variant | Models | Issue |
| --- | --- | --- |
| **Multiface One** (Romantic Robot, ~pcb 2.1) | 48K-class (16K / 48K / Timex) | closed [#137](https://github.com/mward-sudo/spec_chum/issues/137) |
| **Multiface 128** | Spectrum 128 / grey +2 (same `Bus128`; Pentagon accepts attach) | [#168](https://github.com/mward-sudo/spec_chum/issues/168) |

**+2A / +3** are rejected (hardware incompatible). Multiface 3 is not implemented.

## ROM sourcing (user-supplied)

Multiface firmware is **not** redistributed in this repository and is **not** fetched by
`./scripts/fetch_roms.sh` (Romantic Robot; commercial emulator licences are exclusive —
see [ROMS.md](ROMS.md)). Provide an **8 KiB** (8192-byte) `.rom` / `.bin` yourself.

Suggested local layout (gitignored under `roms/`):

| Path | Contents |
| --- | --- |
| `roms/multiface/mf1.rom` | Multiface 1 (late pcb 2.1 preferred) |
| `roms/multiface/mf128.rom` | Multiface 128 |

A personal dump tree outside the git clone (e.g. `…/spec_chum-roms/peripherals/multiface/`)
can be copied into those paths for soak tests. Wrong size is rejected. Empty/zeroed images
are accepted for unit tests but will not show a real Multiface menu.

Attach via:

- **egui:** Hardware → **Attach Multiface 1 ROM…** / **Attach Multiface 128 ROM…** then **Multiface NMI**
- **macOS:** Hardware → **Attach Multiface ROM…** then **Multiface NMI**
  (`sc_attach_multiface` / `sc_multiface_nmi`)
- **API:** `Machine::attach_multiface(&[u8; 8192])` then `Machine::multiface_nmi()`
  (model selects MF1 vs MF128)

## Behaviour modelled

### Multiface 1

| Piece | Behaviour |
| --- | --- |
| Memory | 8 KiB ROM at `0000–1FFF`, 8 KiB RAM at `2000–3FFF` when paged |
| Red button | Asserts NMI pending; Z80 NMI → `0x0066`; vector latch pages MF in |
| `IN` A7=1 (typ. `0x9F`) | Page **in** |
| `IN` A7=0 (typ. `0x1F`) | Page **out** + Kempston joy bits D0–D4 |
| `OUT` (same decode) | Clears NMI pending only (does **not** page out) |
| Reset | Clears paging + NMI pending; **keeps** MF RAM |

Port decode matches Fuse/MAME late MF1: `(port & 0x72) == 0x12`.

### Multiface 128

| Piece | Behaviour |
| --- | --- |
| Memory | Same 8 KiB ROM + 8 KiB RAM overlay when paged |
| Software stealth | Power-up **OFF**; red button / page-in forces **ON**; `OUT` while paged sets ON/OFF from A7 |
| `IN` A7=1 (typ. `0xBF`) | Page **in** when enabled; returns last `#7FFD` screen bit as D7 (`0xFF` / `0x7F`) |
| `IN` A7=0 (typ. `0x3F`) | Page **out** (always) |
| `OUT` (same decode) | Clears NMI pending; if paged, A7 updates software enable |
| Reset | Clears paging, NMI pending, and software enable; **keeps** MF RAM |

Port decode matches Fuse `multiface_ports_128`: `(port & 0x72) == 0x32`.

## Gaps

- No Multiface 3
- No game-level soak in CI (real ROM optional; place under `roms/multiface/`)
- MF1’s onboard Kempston shares the bus Kempston state (no separate stick)
