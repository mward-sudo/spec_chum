//! Timex SCLD ports for TC2048 / TS2068 (#192).
//!
//! Phase 1 (TC2048): latch ports 0xFF and 0xF4 so Timex ROM and BASIC extensions
//! can configure display modes.
//!
//! Phase 2a (TS2068 / TC2068): horizontal MMU uses those latches — bit 7 of
//! port 0xFF selects EX-ROM vs DOCK; port 0xF4 selects which 8K chunks
//! are overlaid on the home bank. AY ports F5/F6 plus R14 Timex joysticks
//! (Fuse-compatible bit layout). Dock cartridges: Warajevo `.dck` via
//! [`crate::TimexDock`].
//!
//! Phase 2b (partial): port `0xFF` screen modes 0–3 drive 256×192 SCLD video
//! (alt display file + hi-colour). Modes 4–7 (512×192 hi-res) remain deferred.

/// Size of the Timex TS2068 / TC2068 EX-ROM bank (chunk 0').
pub const TIMEX_EXROM_SIZE: usize = 8192;

/// Active-high Timex joystick mask (Fuse `timex_mask`): up / down / left / right / fire.
#[must_use]
pub fn timex_joystick_mask(up: bool, down: bool, left: bool, right: bool, fire: bool) -> u8 {
    let mut v = 0u8;
    if up {
        v |= 0x01;
    }
    if down {
        v |= 0x02;
    }
    if left {
        v |= 0x04;
    }
    if right {
        v |= 0x08;
    }
    if fire {
        v |= 0x80;
    }
    v
}

/// Offset of the Timex alternate display file within the 16K screen RAM page
/// (Fuse `ALTDFILE_OFFSET`).
pub const TIMEX_ALTDFILE_OFFSET: usize = 0x2000;

/// Low 3 bits of port `0xFF` — Fuse `scrnmode` (SCLD DEC).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum TimexScreenMode {
    /// Standard Spectrum 256×192 (bitmap + 8×8 attrs at `0x4000`).
    Standard = 0,
    /// Second display file at `+0x2000` (bitmap + 8×8 attrs).
    AltFile = 1,
    /// Hi-colour: primary bitmap, 8×1 attrs from alt file (scrambled).
    ExtColour = 2,
    /// Hi-colour with bitmap + 8×1 attrs both from alt file.
    ExtColourAlt = 3,
    /// 512×192 family (hi-res) — not rendered yet; hosts keep 256×192 path.
    HiresAttr = 4,
    HiresAttrAlt = 5,
    Hires = 6,
    HiresDoubleCol = 7,
}

impl TimexScreenMode {
    #[must_use]
    pub fn from_port_ff(port_ff: u8) -> Self {
        match port_ff & 0x07 {
            1 => Self::AltFile,
            2 => Self::ExtColour,
            3 => Self::ExtColourAlt,
            4 => Self::HiresAttr,
            5 => Self::HiresAttrAlt,
            6 => Self::Hires,
            7 => Self::HiresDoubleCol,
            _ => Self::Standard,
        }
    }

    /// Modes that stay at 256×192 and are drawn by the Phase 2b slice.
    #[must_use]
    pub const fn is_lores_scld(self) -> bool {
        matches!(
            self,
            Self::Standard | Self::AltFile | Self::ExtColour | Self::ExtColourAlt
        )
    }
}

/// Timex SCLD latch state (ports 0xFF and 0xF4).
#[derive(Clone, Debug, Default)]
pub struct TimexScld {
    port_ff: u8,
    port_f4: u8,
}

impl TimexScld {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }

    #[must_use]
    pub fn port_ff(&self) -> u8 {
        self.port_ff
    }

    #[must_use]
    pub fn port_f4(&self) -> u8 {
        self.port_f4
    }

    /// Screen mode from port `0xFF` bits 0–2 (Fuse `scrnmode`).
    #[must_use]
    pub fn screen_mode(&self) -> TimexScreenMode {
        TimexScreenMode::from_port_ff(self.port_ff)
    }

    /// Bit 6 of port `0xFF`: when set, ULA interrupts are inhibited (Fuse `intdisable`).
    #[must_use]
    pub fn int_disabled(&self) -> bool {
        self.port_ff & 0x40 != 0
    }

    /// Bit 7 of port 0xFF: `true` = EX-ROM bank, `false` = DOCK / cartridge.
    #[must_use]
    pub fn use_exrom(&self) -> bool {
        self.port_ff & 0x80 != 0
    }

    /// Whether horizontal-select bit `chunk` (0..7) pages that 8K over home.
    #[must_use]
    pub fn chunk_paged(&self, chunk: u8) -> bool {
        chunk < 8 && self.port_f4 & (1 << chunk) != 0
    }

    /// 8K chunk index for a Z80 address (`addr >> 13`).
    #[must_use]
    pub fn chunk_of(addr: u16) -> u8 {
        (addr >> 13) as u8
    }

    /// Handle IN; returns Some when this port is Timex-decoded.
    #[must_use]
    pub fn in_port(&self, port: u16) -> Option<u8> {
        match port & 0x00FF {
            0x00FF => Some(self.port_ff),
            0x00F4 => Some(self.port_f4),
            _ => None,
        }
    }

    /// Handle OUT; returns true when consumed.
    pub fn out_port(&mut self, port: u16, value: u8) -> bool {
        match port & 0x00FF {
            0x00FF => {
                self.port_ff = value;
                true
            }
            0x00F4 => {
                self.port_f4 = value;
                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joystick_mask_fuse_layout() {
        assert_eq!(timex_joystick_mask(true, false, false, false, false), 0x01);
        assert_eq!(timex_joystick_mask(false, true, false, false, false), 0x02);
        assert_eq!(timex_joystick_mask(false, false, true, false, false), 0x04);
        assert_eq!(timex_joystick_mask(false, false, false, true, false), 0x08);
        assert_eq!(timex_joystick_mask(false, false, false, false, true), 0x80);
    }

    #[test]
    fn port_ff_read_returns_last_write() {
        let mut scld = TimexScld::new();
        assert_eq!(scld.in_port(0x00FF), Some(0));
        scld.out_port(0x00FF, 0x42);
        assert_eq!(scld.in_port(0x00FF), Some(0x42));
    }

    #[test]
    fn port_f4_latches() {
        let mut scld = TimexScld::new();
        scld.out_port(0x00F4, 0x0F);
        assert_eq!(scld.in_port(0x00F4), Some(0x0F));
    }

    #[test]
    fn altmembank_and_chunk_bits() {
        let mut scld = TimexScld::new();
        assert!(!scld.use_exrom());
        assert!(!scld.chunk_paged(0));
        scld.out_port(0x00FF, 0x80);
        scld.out_port(0x00F4, 0x01);
        assert!(scld.use_exrom());
        assert!(scld.chunk_paged(0));
        assert!(!scld.chunk_paged(1));
        assert_eq!(TimexScld::chunk_of(0x1FFF), 0);
        assert_eq!(TimexScld::chunk_of(0x2000), 1);
    }

    #[test]
    fn screen_mode_and_int_disable_from_port_ff() {
        let mut scld = TimexScld::new();
        assert_eq!(scld.screen_mode(), TimexScreenMode::Standard);
        assert!(!scld.int_disabled());
        scld.out_port(0x00FF, 0x02);
        assert_eq!(scld.screen_mode(), TimexScreenMode::ExtColour);
        scld.out_port(0x00FF, 0x46);
        assert_eq!(scld.screen_mode(), TimexScreenMode::Hires);
        assert!(scld.int_disabled());
        assert!(!scld.screen_mode().is_lores_scld());
        assert!(TimexScreenMode::ExtColourAlt.is_lores_scld());
    }
}
