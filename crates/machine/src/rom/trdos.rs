//! TR-DOS ROM image heuristics (5.04 hole / 5.04T VG93 stub).

use bus::TRDOS_ROM_SIZE;

/// True when `08D2h` is Alone Coder / `VfNG` **5.04T** VG93 port-remap stub
/// (`LD A,#2C; JP 0897h`), not a classic TR-DOS file-load service.
#[must_use]
pub fn trdos_rom_08d2_is_vg93_port_stub(data: &[u8]) -> bool {
    data.get(0x08d2..0x08d7) == Some(&[0x3e, 0x2c, 0xc3, 0x97, 0x08][..])
}

/// True when a 16 KiB TR-DOS image has real code in the usual 5.04 hole region.
///
/// Alone Coder **5.04T** fills `0800h`+ with VG93 helpers (including a port stub at
/// `08D2h`); classic file-load at `08D2h` is still absent — see
/// [`trdos_rom_has_native_file_services`].
///
/// Rejects FF padding **and** blank (all-zero) images — probes must be non-zero
/// program bytes, not an empty buffer of the right length.
#[must_use]
pub fn trdos_rom_fills_0800_hole(data: &[u8]) -> bool {
    data.len() == TRDOS_ROM_SIZE
        && data.get(0x08d2).is_some_and(|b| !matches!(*b, 0x00 | 0xff))
        && data.get(0x0d6b).is_some_and(|b| !matches!(*b, 0x00 | 0xff))
}

/// True when a 16 KiB TR-DOS image has a classic RUN file-load service at `08D2h`.
///
/// Many circulating **Ver 5.04** dumps leave `0800h`–`0E71h` as FF padding.
/// Alone Coder / `VfNG` **5.04T** fills that hole with VG93 port remapping (`0897h`
/// trampoline; `08D2h` is `LD A,#2C; JP 0897h`) — not the stock file loader — so
/// post-match `19ECh` still needs the FDC/`LINE-NEW` stand-in for `boot`.
#[must_use]
pub fn trdos_rom_has_native_file_services(data: &[u8]) -> bool {
    trdos_rom_fills_0800_hole(data) && !trdos_rom_08d2_is_vg93_port_stub(data)
}
