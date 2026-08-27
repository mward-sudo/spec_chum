//! ZX Interface 1: shadow ROM paging + Microdrive MDR I/O.
//!
//! Ports (Fuse/MAME partial decode on `A4..A3` = `port & 0x18`):
//! - `0x00` → Microdrive data (`IN`/`OUT` stream bytes under the head)
//! - `0x08` → Control / status (`OUT` motors+RS232; `IN` GAP/SYNC/WPR/…)
//! - `0x10` → Network / RS232 bit (stub: returns idle `0xFF`)
//!
//! ROM paging (MAME/`intf1`): page **in** before opcode fetch at `0x0008` /
//! `0x1708`; page **out** after opcode fetch at `0x0700`. The 8K IF1 ROM is
//! mirrored at `0000–1FFF` and `2000–3FFF` while paged.
//!
//! Microdrive mechanics follow Fuse's byte-level MDR model (not bit-accurate
//! tape timing): COMMS CLK falling edge rotates the motor daisy-chain; with a
//! motor running, data port R/W walks the cartridge image.

use formats::{MdrImage, MDR_SECTORS, MDR_SECTOR_SIZE};
use thiserror::Error;

/// Typical IF1 shadow ROM size (8K).
pub const IF1_ROM_SIZE: usize = 8192;

/// Errors from loading an Interface 1 shadow ROM image.
#[derive(Debug, Error)]
pub enum Interface1RomError {
    #[error("IF1 ROM must be {expected} bytes, got {got}")]
    InvalidSize { expected: usize, got: usize },
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Number of Microdrive units on the daisy chain (hardware max).
pub const MICRODRIVE_COUNT: usize = 8;

/// Header block length inside each 543-byte MDR sector (Fuse `HEAD_LEN`).
const MDR_HEAD_LEN: usize = 15;
/// Data payload + checksum after the header (`HEAD_LEN + DATA_LEN + 1`).
const MDR_DATA_MAX: usize = MDR_HEAD_LEN + 512 + 1;

/// Port class from `port & 0x18` (never `0x18` — that is unclaimed).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum If1Port {
    Mdr,
    Ctr,
    Net,
}

fn decode_port(port: u16) -> Option<If1Port> {
    match port & 0x0018 {
        0x0000 => Some(If1Port::Mdr),
        0x0008 => Some(If1Port::Ctr),
        0x0010 => Some(If1Port::Net),
        _ => None,
    }
}

#[derive(Clone, Debug)]
struct Drive {
    cart: Option<MdrImage>,
    motor_on: bool,
    head_pos: usize,
    transferred: usize,
    max_bytes: usize,
    gap: u8,
    sync: u8,
    last: u8,
    /// Preamble progress / `0xFF` = formatted sync OK (Fuse-style).
    pream: Vec<u8>,
}

impl Drive {
    fn new() -> Self {
        Self {
            cart: None,
            motor_on: false,
            head_pos: 0,
            transferred: 0,
            max_bytes: MDR_HEAD_LEN,
            gap: 15,
            sync: 15,
            last: 0xff,
            pream: vec![0; MDR_SECTORS * 2],
        }
    }

    fn cartridge_len_bytes(cart: &MdrImage) -> usize {
        cart.sectors.len().saturating_mul(MDR_SECTOR_SIZE)
    }

    fn insert(&mut self, cart: MdrImage) {
        let blocks = cart.sectors.len();
        self.pream = vec![0xff; blocks.saturating_mul(2)]; // treat as formatted
        self.cart = Some(cart);
        self.head_pos = 0;
        self.transferred = 0;
        self.gap = 15;
        self.sync = 15;
        self.restart_block();
    }

    fn restart_block(&mut self) {
        let block = MDR_SECTOR_SIZE;
        while !self.head_pos.is_multiple_of(block) && self.head_pos % block != MDR_HEAD_LEN {
            self.increment_head();
        }
        self.transferred = 0;
        self.max_bytes = if self.head_pos.is_multiple_of(block) {
            MDR_HEAD_LEN
        } else {
            MDR_DATA_MAX
        };
    }

    fn increment_head(&mut self) {
        let len = self
            .cart
            .as_ref()
            .map(Self::cartridge_len_bytes)
            .unwrap_or(0);
        if len == 0 {
            self.head_pos = 0;
            return;
        }
        self.head_pos += 1;
        if self.head_pos >= len {
            self.head_pos = 0;
        }
    }

    fn read_byte_at_head(&self) -> u8 {
        let Some(cart) = self.cart.as_ref() else {
            return 0xff;
        };
        let sec = self.head_pos / MDR_SECTOR_SIZE;
        let off = self.head_pos % MDR_SECTOR_SIZE;
        cart.sectors.get(sec).map(|s| s[off]).unwrap_or(0xff)
    }

    fn write_byte_at_head(&mut self, val: u8) -> bool {
        let Some(cart) = self.cart.as_mut() else {
            return false;
        };
        if cart.write_protected {
            return false;
        }
        let sec = self.head_pos / MDR_SECTOR_SIZE;
        let off = self.head_pos % MDR_SECTOR_SIZE;
        if let Some(slot) = cart.sectors.get_mut(sec) {
            slot[off] = val;
            true
        } else {
            false
        }
    }

    fn preamble_ok(&self) -> bool {
        let block = self.head_pos / MDR_SECTOR_SIZE
            + if self.max_bytes == MDR_HEAD_LEN {
                0
            } else {
                MDR_SECTORS
            };
        self.pream.get(block).copied().unwrap_or(0) == 0xff
    }
}

#[derive(Clone, Debug)]
pub struct Interface1 {
    pub rom: [u8; IF1_ROM_SIZE],
    pub rom_paged: bool,
    pub rom_loaded: bool,
    drives: [Drive; MICRODRIVE_COUNT],
    /// Previous COMMS CLK level (bit1 of control OUT).
    comms_clk: bool,
    /// COMMS DATA (bit0) — also selects RS232 vs net on real hardware.
    #[allow(dead_code)]
    comms_data: bool,
    /// Latched control bits (erase / r/w / cts / wait) for inspect/tests.
    pub control: u8,
}

impl Default for Interface1 {
    fn default() -> Self {
        Self::new()
    }
}

impl Interface1 {
    #[must_use]
    pub fn new() -> Self {
        Self {
            rom: [0; IF1_ROM_SIZE],
            rom_paged: false,
            rom_loaded: false,
            drives: std::array::from_fn(|_| Drive::new()),
            comms_clk: false,
            comms_data: false,
            control: 0,
        }
    }

    pub fn load_rom(&mut self, data: &[u8]) -> Result<(), Interface1RomError> {
        if data.len() != IF1_ROM_SIZE {
            return Err(Interface1RomError::InvalidSize {
                expected: IF1_ROM_SIZE,
                got: data.len(),
            });
        }
        self.rom.copy_from_slice(data);
        self.rom_loaded = true;
        Ok(())
    }

    pub fn load_rom_path(&mut self, path: &std::path::Path) -> Result<(), Interface1RomError> {
        let data = std::fs::read(path)?;
        self.load_rom(&data)
    }

    pub fn page_rom(&mut self, on: bool) {
        self.rom_paged = on;
    }

    /// Insert cartridge into drive `0` (UI / smoke path).
    pub fn insert_mdr(&mut self, cart: MdrImage) {
        self.insert_mdr_drive(0, cart);
    }

    /// Insert cartridge into drive `index` (`0..MICRODRIVE_COUNT`).
    pub fn insert_mdr_drive(&mut self, index: usize, cart: MdrImage) {
        if let Some(d) = self.drives.get_mut(index) {
            d.insert(cart);
        }
    }

    /// Drive 0 cartridge (compat with earlier stub API).
    #[must_use]
    pub fn mdr(&self) -> Option<&MdrImage> {
        self.drives[0].cart.as_ref()
    }

    pub fn mdr_mut(&mut self) -> Option<&mut MdrImage> {
        self.drives[0].cart.as_mut()
    }

    /// Whether any drive motor is running.
    #[must_use]
    pub fn any_motor_on(&self) -> bool {
        self.drives.iter().any(|d| d.motor_on)
    }

    /// Page in before opcode fetch at `0x0008` / `0x1708`.
    pub fn pre_opcode_fetch(&mut self, pc: u16) {
        if self.rom_loaded && matches!(pc, 0x0008 | 0x1708) {
            self.rom_paged = true;
        }
    }

    /// Page out after opcode fetch at `0x0700` (must see IF1 ROM byte first).
    pub fn post_opcode_fetch(&mut self, pc: u16) {
        if pc == 0x0700 {
            self.rom_paged = false;
        }
    }

    /// Shadow ROM visible at `0000–3FFF` when paged and a ROM image is loaded.
    #[must_use]
    pub fn read_rom(&self, addr: u16) -> Option<u8> {
        if !self.rom_loaded || !self.rom_paged || addr >= 0x4000 {
            return None;
        }
        Some(self.rom[(addr as usize) & (IF1_ROM_SIZE - 1)])
    }

    /// `OUT` to IF1 ports. Returns true if handled.
    pub fn out_port(&mut self, port: u16, value: u8) -> bool {
        match decode_port(port) {
            Some(If1Port::Mdr) => {
                self.port_mdr_out(value);
                true
            }
            Some(If1Port::Ctr) => {
                self.port_ctr_out(value);
                true
            }
            Some(If1Port::Net) => true, // RS232/net TX stub
            None => false,
        }
    }

    /// `IN` from IF1 ports.
    pub fn in_port(&mut self, port: u16) -> Option<u8> {
        match decode_port(port) {
            Some(If1Port::Mdr) => Some(self.port_mdr_in()),
            Some(If1Port::Ctr) => Some(self.port_ctr_in()),
            Some(If1Port::Net) => Some(0xff), // idle net / no RS232
            None => None,
        }
    }

    fn port_mdr_in(&mut self) -> u8 {
        let mut ret = 0xff;
        for d in &mut self.drives {
            if !(d.motor_on && d.cart.is_some()) {
                continue;
            }
            if d.transferred < d.max_bytes {
                d.last = d.read_byte_at_head();
                d.increment_head();
            }
            d.transferred = d.transferred.saturating_add(1);
            ret &= d.last;
        }
        ret
    }

    fn port_mdr_out(&mut self, val: u8) {
        for d in &mut self.drives {
            if !(d.motor_on && d.cart.is_some()) {
                continue;
            }
            let block_idx = d.head_pos / MDR_SECTOR_SIZE
                + if d.max_bytes == MDR_HEAD_LEN {
                    0
                } else {
                    MDR_SECTORS
                };
            if d.transferred == 0 && val == 0x00 {
                if let Some(p) = d.pream.get_mut(block_idx) {
                    *p = 1;
                }
            } else if ((1..10).contains(&d.transferred) && val == 0x00)
                || ((10..12).contains(&d.transferred) && val == 0xff)
            {
                if let Some(p) = d.pream.get_mut(block_idx) {
                    *p = p.saturating_add(1);
                }
            } else if d.transferred == 12 && d.pream.get(block_idx).copied().unwrap_or(0) == 12 {
                if let Some(p) = d.pream.get_mut(block_idx) {
                    *p = 0xff;
                }
            }
            if d.transferred > 11 && d.transferred < d.max_bytes.saturating_add(12) {
                let _ = d.write_byte_at_head(val);
                d.increment_head();
            }
            d.transferred = d.transferred.saturating_add(1);
        }
    }

    fn port_ctr_in(&mut self) -> u8 {
        let mut ret = 0xff;
        for d in &mut self.drives {
            if !(d.motor_on && d.cart.is_some()) {
                continue;
            }
            if d.preamble_ok() {
                if d.gap > 0 {
                    d.gap -= 1;
                } else {
                    ret &= 0xf9; // GAP + SYNC low
                    if d.sync > 0 {
                        d.sync -= 1;
                    } else {
                        d.gap = 15;
                        d.sync = 15;
                    }
                }
            }
            if d.cart.as_ref().is_some_and(|c| c.write_protected) {
                ret &= 0xfe; // WPR active-low
            }
        }
        self.restart_drives();
        ret
    }

    fn port_ctr_out(&mut self, val: u8) {
        let clk = val & 0x02 != 0;
        // Falling edge of COMMS CLK: shift motor daisy-chain; new drive0 = !COMMS_DATA.
        if !clk && self.comms_clk {
            for m in (1..MICRODRIVE_COUNT).rev() {
                self.drives[m].motor_on = self.drives[m - 1].motor_on;
            }
            self.drives[0].motor_on = val & 0x01 == 0;
        }
        self.comms_data = val & 0x01 != 0;
        self.comms_clk = clk;
        self.control = val;
        self.restart_drives();
    }

    fn restart_drives(&mut self) {
        for d in &mut self.drives {
            d.restart_block();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mdr_roundtrip_via_if1() {
        let mut if1 = Interface1::new();
        let mut cart = MdrImage::blank();
        cart.write_sector(3, &[0xaa, 0xbb]).unwrap();
        if1.insert_mdr(cart);
        assert_eq!(if1.mdr().unwrap().read_sector(3).unwrap()[0], 0xaa);
    }

    #[test]
    fn shadow_rom_page_and_mirror() {
        let mut if1 = Interface1::new();
        let mut rom = [0u8; IF1_ROM_SIZE];
        rom[0x10] = 0x55;
        if1.load_rom(&rom).unwrap();
        assert!(if1.read_rom(0x10).is_none());
        if1.page_rom(true);
        assert_eq!(if1.read_rom(0x10), Some(0x55));
        assert_eq!(if1.read_rom(0x2010), Some(0x55));
    }

    #[test]
    fn unloaded_rom_never_shadows() {
        let mut if1 = Interface1::new();
        if1.page_rom(true);
        assert!(if1.read_rom(0x10).is_none());
        if1.page_rom(false);
        if1.pre_opcode_fetch(0x0008);
        assert!(!if1.rom_paged);
        assert!(if1.read_rom(0x10).is_none());
    }

    #[test]
    fn rom_paging_hooks_at_classic_addresses() {
        let mut if1 = Interface1::new();
        if1.load_rom(&[0u8; IF1_ROM_SIZE]).unwrap();
        if1.pre_opcode_fetch(0x0008);
        assert!(if1.rom_paged);
        if1.post_opcode_fetch(0x0700);
        assert!(!if1.rom_paged);
        if1.pre_opcode_fetch(0x1708);
        assert!(if1.rom_paged);
    }

    #[test]
    fn motor_select_and_sector_stream_read() {
        let mut if1 = Interface1::new();
        let mut cart = MdrImage::blank();
        // Put a recognizable byte at the start of sector 0 (header region).
        cart.sectors[0][0] = 0x5a;
        cart.sectors[0][1] = 0xa5;
        if1.insert_mdr(cart);

        // Select drive 0: COMMS_DATA=0, pulse COMMS_CLK high→low.
        assert!(if1.out_port(0x00ef, 0x02)); // clk=1, data=0
        assert!(if1.out_port(0x00ef, 0x00)); // falling edge → motor0 on
        assert!(if1.any_motor_on());

        // Status poll restarts block alignment to sector start.
        let _ = if1.in_port(0x00ef);

        let b0 = if1.in_port(0x00e7).unwrap();
        let b1 = if1.in_port(0x00e7).unwrap();
        assert_eq!(b0, 0x5a);
        assert_eq!(b1, 0xa5);
    }

    #[test]
    fn motor_select_and_sector_stream_write() {
        let mut if1 = Interface1::new();
        if1.insert_mdr(MdrImage::blank());
        if1.out_port(0x00ef, 0x02);
        if1.out_port(0x00ef, 0x00);
        let _ = if1.in_port(0x00ef);

        // Skip preamble bookkeeping: transferred advances; write path starts after 11.
        for _ in 0..12 {
            if1.out_port(0x00e7, 0x00);
        }
        // After 12 preamble-ish outs, next outs write into the cartridge.
        if1.out_port(0x00e7, 0x11);
        if1.out_port(0x00e7, 0x22);

        let cart = if1.mdr().unwrap();
        // Head advanced during preamble outs + writes; at least one written byte lands.
        let flat: Vec<u8> = cart
            .sectors
            .iter()
            .flat_map(|s| s.iter().copied())
            .collect();
        assert!(
            flat.contains(&0x11) && flat.contains(&0x22),
            "expected written bytes in MDR image"
        );
    }

    #[test]
    fn write_protect_clears_status_bit0() {
        let mut if1 = Interface1::new();
        let mut cart = MdrImage::blank();
        cart.write_protected = true;
        if1.insert_mdr(cart);
        if1.out_port(0x00ef, 0x02);
        if1.out_port(0x00ef, 0x00);
        let st = if1.in_port(0x00ef).unwrap();
        assert_eq!(st & 1, 0, "WPR should pull bit0 low");
    }

    #[test]
    fn port_decode_ignores_unclaimed_mask() {
        let mut if1 = Interface1::new();
        assert!(!if1.out_port(0x0018, 0));
        assert!(if1.in_port(0x0018).is_none());
    }
}
