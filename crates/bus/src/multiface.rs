//! Multiface 1 (48K) and Multiface 128 (128K / grey +2): 8K ROM + 8K RAM overlay
//! at `0000–3FFF`, red-button NMI.
//!
//! ## Multiface One (late pcb ~2.1 / Fuse + MAME `mface1`)
//! - Red button asserts NMI pending; the NMI vector fetch pages MF memory in.
//! - `IN` on ports matching `xxxx xxxx x001 xx1x` pages by **A7**:
//!   - A7=1 (typ. `0x9F`) → page **in**
//!   - A7=0 (typ. `0x1F`) → page **out** (also returns Kempston-style joy bits)
//! - `OUT` on the same decode clears NMI pending (does not itself page out).
//!
//! ## Multiface 128 (Fuse `multiface_ports_128`)
//! - Works on Spectrum 128 / grey +2 (**not** +2A/+3).
//! - Port decode `(port & 0x72) == 0x32` (typ. `IN 0xBF` page in, `IN 0x3F` page out).
//! - Software stealth (`J2`): power-up **OFF**; paging in (NMI or `IN` while ON)
//!   forces **ON**; `OUT` while paged sets ON/OFF from A7.
//! - Page-in `IN` returns last `#7FFD` screen bit as D7 (`0xFF` / `0x7F`).
//!
//! Real Multiface ROM images are **not** shipped in-tree — attach via bytes/path
//! like system ROMs (see `docs/MULTIFACE.md`).

use std::path::Path;

use crate::RomLoadError;

/// Multiface One / 128 memory size (ROM and RAM each).
pub const MULTIFACE1_SIZE: usize = 8192;

/// Alias — Multiface 128 also uses an 8 KiB ROM/RAM pair.
pub const MULTIFACE128_SIZE: usize = MULTIFACE1_SIZE;

/// Fuse / late MF1 I/O decode: `(port & 0x72) == 0x12` (e.g. `0x1F`, `0x9F`).
#[inline]
#[must_use]
pub fn multiface1_port_match(port: u16) -> bool {
    port & 0x0072 == 0x0012
}

/// Fuse Multiface 128 I/O decode: `(port & 0x72) == 0x32` (e.g. `0x3F`, `0xBF`).
#[inline]
#[must_use]
pub fn multiface128_port_match(port: u16) -> bool {
    port & 0x0072 == 0x0032
}

/// Romantic Instruments Multiface 1 overlay for 48K machines.
#[derive(Clone, Debug)]
pub struct Multiface1 {
    pub rom: [u8; MULTIFACE1_SIZE],
    pub ram: [u8; MULTIFACE1_SIZE],
    /// True while MF ROM/RAM overlay Spectrum `0000–3FFF`.
    pub paged: bool,
    /// Red-button NMI line held until cleared by a matching `OUT`.
    pub nmi_pending: bool,
    /// Set when a ROM image has been attached (empty ROM is still valid for tests).
    pub rom_loaded: bool,
}

impl Default for Multiface1 {
    fn default() -> Self {
        Self::new()
    }
}

impl Multiface1 {
    #[must_use]
    pub fn new() -> Self {
        Self {
            rom: [0; MULTIFACE1_SIZE],
            ram: [0; MULTIFACE1_SIZE],
            paged: false,
            nmi_pending: false,
            rom_loaded: false,
        }
    }

    /// Load an 8 KiB Multiface ROM image from memory.
    pub fn load_rom(&mut self, data: &[u8]) -> Result<(), RomLoadError> {
        if data.len() != MULTIFACE1_SIZE {
            return Err(RomLoadError::WrongSize {
                kind: "Multiface 1 ROM",
                expected: MULTIFACE1_SIZE,
                got: data.len(),
            });
        }
        self.rom.copy_from_slice(data);
        self.rom_loaded = true;
        Ok(())
    }

    /// Load Multiface ROM from a filesystem path.
    pub fn load_rom_path(&mut self, path: &Path) -> Result<(), RomLoadError> {
        let data = std::fs::read(path)?;
        self.load_rom(&data)
    }

    /// Red button: assert NMI pending (caller raises Z80 NMI). Idempotent while held.
    ///
    /// Returns `true` if this call newly asserted pending (caller should pulse NMI).
    pub fn press_button(&mut self) -> bool {
        if self.nmi_pending {
            return false;
        }
        self.nmi_pending = true;
        true
    }

    /// Alias used by host paths: button + expect caller to raise NMI then
    /// [`Self::page_on_nmi_vector`].
    pub fn nmi(&mut self) {
        let _ = self.press_button();
    }

    /// Page MF in when the CPU takes the NMI vector (`0x0066` / `0x0067`) while pending.
    ///
    /// Call after `cpu.nmi()` (or from an M1 fetch of those addresses). Real hardware
    /// latches `ROMCS` on the vector opcode fetch; this matches that timing for hosts
    /// that raise NMI in one shot.
    pub fn page_on_nmi_vector(&mut self) {
        if self.nmi_pending {
            self.paged = true;
        }
    }

    /// Soft page-in (same as `IN` with A7=1).
    pub fn page_in(&mut self) {
        self.paged = true;
    }

    /// Hide Multiface memory (restore Spectrum ROM at `0000`).
    pub fn hide(&mut self) {
        self.paged = false;
    }

    /// Machine reset: clear overlay and NMI pending (MF RAM contents are kept).
    pub fn reset(&mut self) {
        self.paged = false;
        self.nmi_pending = false;
    }

    /// `IN` on MF1 ports: page by A7; return Kempston bits on page-out ports.
    ///
    /// `joy` is the Kempston-compatible value (D0–D4) when the port is a page-out
    /// decode (A7=0). Page-in ports return `0xff` (floating / unused data).
    ///
    /// Callers that chain expansion devices should apply this for its paging side
    /// effect, then prefer another device’s read data when that device claims the
    /// port (e.g. Beta Disk status on `0x1f` while TR-DOS is paged).
    #[must_use]
    pub fn in_port(&mut self, port: u16, joy: u8) -> Option<u8> {
        if !multiface1_port_match(port) {
            return None;
        }
        if port & 0x0080 != 0 {
            // A7=1 → page in (e.g. IN A,(159) / 0x9F)
            self.paged = true;
            Some(0xff)
        } else {
            // A7=0 → page out (e.g. IN A,(31) / 0x1F) + joystick
            self.paged = false;
            Some(joy & 0x1f)
        }
    }

    /// `OUT` on MF1 ports clears NMI pending (does not page out).
    ///
    /// Returns `true` if the port matched. Callers that chain expansion devices
    /// (e.g. [`crate::Bus48`]) should apply this side effect and continue dispatch
    /// rather than treating a match as exclusive ownership of the cycle.
    #[must_use]
    pub fn out_port(&mut self, port: u16, _value: u8) -> bool {
        if !multiface1_port_match(port) {
            return false;
        }
        self.nmi_pending = false;
        true
    }

    /// Read through the MF overlay when paged; `None` if MF does not own `addr`.
    #[must_use]
    pub fn read(&self, addr: u16) -> Option<u8> {
        if !self.paged {
            return None;
        }
        match addr {
            0x0000..=0x1fff => Some(self.rom[addr as usize]),
            0x2000..=0x3fff => Some(self.ram[addr as usize - 0x2000]),
            _ => None,
        }
    }

    /// Write to MF RAM when paged; returns true if handled.
    pub fn write(&mut self, addr: u16, value: u8) -> bool {
        if !self.paged {
            return false;
        }
        if (0x2000..=0x3fff).contains(&addr) {
            self.ram[addr as usize - 0x2000] = value;
            return true;
        }
        // ROM region ignores writes while paged.
        addr < 0x2000
    }
}

/// Romantic Robot Multiface 128 overlay for Spectrum 128 / grey +2.
#[derive(Clone, Debug)]
pub struct Multiface128 {
    pub rom: [u8; MULTIFACE128_SIZE],
    pub ram: [u8; MULTIFACE128_SIZE],
    /// True while MF ROM/RAM overlay Spectrum `0000–3FFF`.
    pub paged: bool,
    /// Red-button NMI line held until cleared by a matching `OUT`.
    pub nmi_pending: bool,
    /// Software stealth / enable (`J2` in Fuse): `false` = invisible to software
    /// paging until the red button (or an enabled page-in) turns it on.
    pub enabled: bool,
    /// Set when a ROM image has been attached (empty ROM is still valid for tests).
    pub rom_loaded: bool,
}

impl Default for Multiface128 {
    fn default() -> Self {
        Self::new()
    }
}

impl Multiface128 {
    #[must_use]
    pub fn new() -> Self {
        Self {
            rom: [0; MULTIFACE128_SIZE],
            ram: [0; MULTIFACE128_SIZE],
            paged: false,
            nmi_pending: false,
            // Power-up: Multiface 128 is OFF (manual / Fuse reset).
            enabled: false,
            rom_loaded: false,
        }
    }

    /// Load an 8 KiB Multiface 128 ROM image from memory.
    pub fn load_rom(&mut self, data: &[u8]) -> Result<(), RomLoadError> {
        if data.len() != MULTIFACE128_SIZE {
            return Err(RomLoadError::WrongSize {
                kind: "Multiface 128 ROM",
                expected: MULTIFACE128_SIZE,
                got: data.len(),
            });
        }
        self.rom.copy_from_slice(data);
        self.rom_loaded = true;
        Ok(())
    }

    /// Load Multiface 128 ROM from a filesystem path.
    pub fn load_rom_path(&mut self, path: &Path) -> Result<(), RomLoadError> {
        let data = std::fs::read(path)?;
        self.load_rom(&data)
    }

    /// Red button: assert NMI pending (caller raises Z80 NMI). Idempotent while held.
    ///
    /// Returns `true` if this call newly asserted pending (caller should pulse NMI).
    /// Unlike MF1 stealth jumper, MF128 always accepts the red button.
    pub fn press_button(&mut self) -> bool {
        if self.nmi_pending {
            return false;
        }
        self.nmi_pending = true;
        true
    }

    /// Alias used by host paths: button + expect caller to raise NMI then
    /// [`Self::page_on_nmi_vector`].
    pub fn nmi(&mut self) {
        let _ = self.press_button();
    }

    /// Page MF in when the CPU takes the NMI vector while pending.
    ///
    /// Also forces software enable ON (red button activates *and* switches ON).
    pub fn page_on_nmi_vector(&mut self) {
        if self.nmi_pending {
            self.page_in();
        }
    }

    /// Soft page-in (same as enabled `IN` with A7=1). Forces enable ON.
    pub fn page_in(&mut self) {
        self.paged = true;
        self.enabled = true;
    }

    /// Hide Multiface memory (restore Spectrum ROM at `0000`).
    pub fn hide(&mut self) {
        self.paged = false;
    }

    /// Machine reset: clear overlay, NMI pending, and software enable (MF RAM kept).
    pub fn reset(&mut self) {
        self.paged = false;
        self.nmi_pending = false;
        self.enabled = false;
    }

    /// `IN` on MF128 ports: page by A7 when decode matches.
    ///
    /// `last_7ffd` is the last value written to `#7FFD` (screen bit D3 → return D7).
    /// Page-out ports always unpage; page-in only when [`Self::enabled`].
    #[must_use]
    pub fn in_port(&mut self, port: u16, last_7ffd: u8) -> Option<u8> {
        if !multiface128_port_match(port) {
            return None;
        }
        if port & 0x0080 != 0 {
            // A7=1 → page in when enabled (e.g. IN A,(191) / 0xBF)
            if self.enabled {
                self.page_in();
                // D7 reflects last screen select (bit 3 of #7FFD).
                Some(if last_7ffd & 0x08 != 0 { 0xff } else { 0x7f })
            } else {
                Some(0xff)
            }
        } else {
            // A7=0 → page out (e.g. IN A,(63) / 0x3F)
            self.paged = false;
            Some(0xff)
        }
    }

    /// `OUT` on MF128 ports: clear NMI pending; when paged, A7 sets software enable.
    #[must_use]
    pub fn out_port(&mut self, port: u16, _value: u8) -> bool {
        if !multiface128_port_match(port) {
            return false;
        }
        if self.paged {
            self.enabled = port & 0x0080 != 0;
        }
        self.nmi_pending = false;
        true
    }

    /// Read through the MF overlay when paged; `None` if MF does not own `addr`.
    #[must_use]
    pub fn read(&self, addr: u16) -> Option<u8> {
        if !self.paged {
            return None;
        }
        match addr {
            0x0000..=0x1fff => Some(self.rom[addr as usize]),
            0x2000..=0x3fff => Some(self.ram[addr as usize - 0x2000]),
            _ => None,
        }
    }

    /// Write to MF RAM when paged; returns true if handled.
    pub fn write(&mut self, addr: u16, value: u8) -> bool {
        if !self.paged {
            return false;
        }
        if (0x2000..=0x3fff).contains(&addr) {
            self.ram[addr as usize - 0x2000] = value;
            return true;
        }
        addr < 0x2000
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn button_pages_on_nmi_vector() {
        let mut mf = Multiface1::new();
        mf.rom[0x66] = 0x3e;
        assert!(mf.press_button());
        assert!(mf.nmi_pending);
        assert!(!mf.paged);
        assert!(mf.read(0x0066).is_none());
        mf.page_on_nmi_vector();
        assert!(mf.paged);
        assert_eq!(mf.read(0x0066), Some(0x3e));
        assert!(mf.write(0x2000, 0xaa));
        assert_eq!(mf.read(0x2000), Some(0xaa));
        // Second press while pending is ignored.
        assert!(!mf.press_button());
    }

    #[test]
    fn in_9f_pages_in_in_1f_pages_out() {
        let mut mf = Multiface1::new();
        mf.rom[0] = 0xc3;
        assert_eq!(mf.in_port(0x009f, 0), Some(0xff));
        assert!(mf.paged);
        assert_eq!(mf.read(0x0000), Some(0xc3));
        assert_eq!(mf.in_port(0x001f, 0x11), Some(0x11));
        assert!(!mf.paged);
        assert!(mf.read(0x0000).is_none());
    }

    #[test]
    fn out_1f_clears_nmi_pending_without_unpaging() {
        let mut mf = Multiface1::new();
        mf.nmi();
        mf.page_on_nmi_vector();
        assert!(mf.paged);
        assert!(mf.nmi_pending);
        assert!(mf.out_port(0x001f, 0));
        assert!(!mf.nmi_pending);
        assert!(mf.paged, "OUT clears NMI only; IN pages out");
    }

    #[test]
    fn out_3f_is_not_mf1_decode() {
        let mut mf = Multiface1::new();
        mf.nmi();
        mf.page_on_nmi_vector();
        assert!(!mf.out_port(0x003f, 0), "late MF1 does not decode 0x3F");
        assert!(mf.nmi_pending);
        assert!(mf.paged);
    }

    #[test]
    fn port_match_covers_1f_and_9f() {
        assert!(multiface1_port_match(0x001f));
        assert!(multiface1_port_match(0x009f));
        assert!(multiface1_port_match(0xff9f));
        assert!(!multiface1_port_match(0x003f));
        assert!(!multiface1_port_match(0x00bf));
        assert!(!multiface1_port_match(0x00fe));
    }

    #[test]
    fn reset_clears_paging_keeps_ram() {
        let mut mf = Multiface1::new();
        mf.page_in();
        mf.ram[0] = 0x5a;
        mf.nmi_pending = true;
        mf.reset();
        assert!(!mf.paged);
        assert!(!mf.nmi_pending);
        assert_eq!(mf.ram[0], 0x5a);
    }

    #[test]
    fn load_rom_size_check() {
        let mut mf = Multiface1::new();
        assert!(mf.load_rom(&[0u8; 10]).is_err());
        assert!(mf.load_rom(&[0u8; MULTIFACE1_SIZE]).is_ok());
        assert!(mf.rom_loaded);
    }

    #[test]
    fn save_return_style_page_toggle() {
        // MF toolkit pages out to run Spectrum code, then IN 9Fh to return.
        let mut mf = Multiface1::new();
        mf.rom[0x100] = 0xc9; // RET stub
        mf.page_in();
        assert!(mf.write(0x2000, 0x42));
        assert_eq!(mf.in_port(0x001f, 0), Some(0));
        assert!(!mf.paged);
        assert_eq!(mf.in_port(0x009f, 0), Some(0xff));
        assert!(mf.paged);
        assert_eq!(mf.read(0x2000), Some(0x42));
        assert_eq!(mf.read(0x0100), Some(0xc9));
    }

    #[test]
    fn mf128_port_match_covers_bf_and_3f() {
        assert!(multiface128_port_match(0x00bf));
        assert!(multiface128_port_match(0x003f));
        assert!(multiface128_port_match(0xffbf));
        assert!(!multiface128_port_match(0x009f), "MF1 0x9F is not MF128");
        assert!(!multiface128_port_match(0x001f));
        assert!(!multiface128_port_match(0x00fe));
    }

    #[test]
    fn mf128_power_up_stealth_blocks_page_in_until_nmi() {
        let mut mf = Multiface128::new();
        mf.rom[0] = 0xc3;
        assert!(!mf.enabled);
        assert_eq!(mf.in_port(0x00bf, 0), Some(0xff));
        assert!(!mf.paged, "stealth OFF blocks software page-in");
        assert!(mf.press_button());
        mf.page_on_nmi_vector();
        assert!(mf.paged);
        assert!(mf.enabled);
        assert_eq!(mf.read(0x0000), Some(0xc3));
    }

    #[test]
    fn mf128_in_bf_returns_screen_bit_and_in_3f_pages_out() {
        let mut mf = Multiface128::new();
        mf.enabled = true;
        mf.rom[0] = 0xed;
        assert_eq!(mf.in_port(0x00bf, 0x00), Some(0x7f));
        assert!(mf.paged);
        assert_eq!(mf.in_port(0x00bf, 0x08), Some(0xff));
        assert_eq!(mf.in_port(0x003f, 0), Some(0xff));
        assert!(!mf.paged);
    }

    #[test]
    fn mf128_out_while_paged_sets_software_enable() {
        let mut mf = Multiface128::new();
        mf.nmi();
        mf.page_on_nmi_vector();
        assert!(mf.enabled);
        assert!(mf.out_port(0x003f, 0), "OUT 3Fh clears NMI + disables");
        assert!(!mf.nmi_pending);
        assert!(mf.paged, "OUT does not unpage");
        assert!(!mf.enabled);
        // Page out, then soft page-in blocked until re-enabled.
        assert_eq!(mf.in_port(0x003f, 0), Some(0xff));
        assert!(!mf.paged);
        assert_eq!(mf.in_port(0x00bf, 0), Some(0xff));
        assert!(!mf.paged);
        // Re-page via NMI then OUT BFh keeps enable ON.
        assert!(mf.press_button());
        mf.page_on_nmi_vector();
        assert!(mf.out_port(0x00bf, 0));
        assert!(mf.enabled);
    }

    #[test]
    fn mf128_reset_clears_enable_keeps_ram() {
        let mut mf = Multiface128::new();
        mf.page_in();
        mf.ram[0] = 0x5a;
        mf.nmi_pending = true;
        mf.reset();
        assert!(!mf.paged);
        assert!(!mf.nmi_pending);
        assert!(!mf.enabled);
        assert_eq!(mf.ram[0], 0x5a);
    }
}
