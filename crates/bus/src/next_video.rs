//! Spectrum Next's Layer 2 state and shared display palettes.

const PAGE_SIZE: usize = 0x2000;
const BANK_SIZE: usize = 0x4000;

fn expand_rgb3(value: u8) -> u8 {
    ((u16::from(value & 0x07) * 255) / 7) as u8
}

#[derive(Clone, Copy, Debug)]
struct PaletteEntry {
    rrr_ggg_bb: u8,
    priority_and_blue: u8,
}

impl PaletteEntry {
    fn rgb332(index: u8) -> Self {
        Self {
            rrr_ggg_bb: index,
            priority_and_blue: u8::from(index & 0x03 != 0),
        }
    }

    fn default_rgb332_palette() -> [Self; 256] {
        std::array::from_fn(|index| Self::rgb332(index as u8))
    }

    fn rgb(self) -> [u8; 3] {
        let blue = (self.rrr_ggg_bb & 0x03) | ((self.priority_and_blue & 0x01) << 2);
        [
            expand_rgb3(self.rrr_ggg_bb >> 5),
            expand_rgb3((self.rrr_ggg_bb >> 2) & 0x07),
            expand_rgb3(blue),
        ]
    }

    fn priority(self) -> bool {
        self.priority_and_blue & 0x80 != 0
    }

    fn transparent(self, transparency: u8) -> bool {
        self.rrr_ggg_bb == transparency
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// RGB output and the layer metadata needed for frame composition.
pub struct VideoColor {
    /// Red, green, and blue intensities in the range 0–255.
    pub rgb: [u8; 3],
    /// Whether this is a Layer 2 color programmed to override other layers.
    pub priority: bool,
    /// Whether the source pixel should reveal a layer underneath it.
    pub transparent: bool,
}

#[derive(Clone, Debug)]
pub(super) struct NextVideo {
    first_bank: u8,
    control: u8,
    visible: bool,
    priority: u8,
    transparency: u8,
    palette_control: u8,
    palette_index: u8,
    palette_write_9bit: bool,
    palette_pending_first_byte: u8,
    palettes: [[PaletteEntry; 256]; 2],
    sprite_palettes: [[PaletteEntry; 256]; 2],
}

impl NextVideo {
    pub(super) fn new() -> Self {
        Self {
            first_bank: 8,
            control: 0,
            visible: false,
            priority: 0,
            transparency: 0xe3,
            palette_control: 0,
            palette_index: 0,
            palette_write_9bit: false,
            palette_pending_first_byte: 0,
            palettes: [PaletteEntry::default_rgb332_palette(); 2],
            sprite_palettes: [PaletteEntry::default_rgb332_palette(); 2],
        }
    }

    pub(super) fn reset(&mut self) {
        *self = Self::new();
    }

    pub(super) fn reset_preserving_palette(&mut self) {
        let palettes = self.palettes;
        let sprite_palettes = self.sprite_palettes;
        self.reset();
        self.palettes = palettes;
        self.sprite_palettes = sprite_palettes;
    }

    pub(super) fn read_register(&self, register: u8) -> Option<u8> {
        match register {
            0x12 => Some(self.first_bank),
            0x14 => Some(self.transparency),
            0x15 => Some(self.priority),
            0x69 => Some(u8::from(self.visible) << 7),
            0x70 => Some(self.control),
            _ => None,
        }
    }

    pub(super) fn write_register(&mut self, register: u8, value: u8) -> bool {
        match register {
            0x12 => self.first_bank = value & 0x7f,
            0x14 => self.transparency = value,
            0x15 => self.priority = value,
            0x69 => self.visible = value & 0x80 != 0,
            0x70 => self.control = value & 0x3f,
            _ => return false,
        }
        true
    }

    /// Keep the visibility bit mirrored by the Layer 2 control I/O port.
    pub(super) fn write_visibility_port(&mut self, value: u8) {
        if value & 0x10 == 0 {
            self.visible = value & 0x02 != 0;
        }
    }

    pub(super) fn standard_mode(&self) -> bool {
        self.control & 0x30 == 0
    }

    pub(super) fn layer2_pixel(&self, ram: &[u8], x: usize, y: usize) -> u8 {
        if x >= 256 || y >= 192 || self.control & 0x30 != 0 {
            return 0;
        }
        let offset = y * 256 + x;
        let bank = usize::from(self.first_bank) + offset / BANK_SIZE;
        let bank_offset = offset % BANK_SIZE;
        let page = bank * 2 + bank_offset / PAGE_SIZE;
        let offset = page * PAGE_SIZE + bank_offset % PAGE_SIZE;
        ram.get(offset).copied().unwrap_or(0)
    }

    pub(super) fn layer2_color(&self, index: u8) -> VideoColor {
        let entry = self.active_palette()[usize::from(index)];
        VideoColor {
            rgb: entry.rgb(),
            priority: entry.priority(),
            transparent: entry.transparent(self.transparency),
        }
    }

    pub(super) fn sprite_color(
        &self,
        pixel: u8,
        palette_offset: u8,
        transparency: u8,
    ) -> VideoColor {
        let index = pixel.wrapping_add(palette_offset << 4);
        let entry = self.active_sprite_palette()[usize::from(index)];
        VideoColor {
            rgb: entry.rgb(),
            priority: false,
            transparent: pixel == transparency,
        }
    }

    pub(super) fn palette_register(&self, register: u8) -> Option<u8> {
        match register {
            0x40 => Some(self.palette_index),
            0x41 => Some(
                self.read_write_palette_entry()
                    .map_or(0xff, |entry| entry.rrr_ggg_bb),
            ),
            0x43 => Some(self.palette_control),
            0x44 => Some(
                self.read_write_palette_entry()
                    .map_or(0xff, |entry| entry.priority_and_blue),
            ),
            _ => None,
        }
    }

    pub(super) fn write_palette_register(&mut self, register: u8, value: u8) -> bool {
        match register {
            0x40 => {
                self.palette_index = value;
                self.palette_write_9bit = false;
            }
            0x41 => self.write_palette_8(value),
            0x43 => {
                self.palette_control = value;
                self.palette_write_9bit = false;
            }
            0x44 => self.write_palette_9(value),
            _ => return false,
        }
        true
    }

    pub(super) fn palette_second_write_pending(&self) -> bool {
        self.palette_write_9bit
    }

    pub(super) fn visible(&self) -> bool {
        self.visible
    }

    pub(super) fn priority(&self) -> u8 {
        self.priority
    }

    fn active_palette(&self) -> &[PaletteEntry; 256] {
        &self.palettes[usize::from(self.palette_control & 0x04 != 0)]
    }

    fn active_sprite_palette(&self) -> &[PaletteEntry; 256] {
        &self.sprite_palettes[usize::from(self.palette_control & 0x08 != 0)]
    }

    fn read_write_palette(&self) -> &[PaletteEntry; 256] {
        match (self.palette_control >> 4) & 0x07 {
            1 => &self.palettes[0],
            5 => &self.palettes[1],
            2 => &self.sprite_palettes[0],
            6 => &self.sprite_palettes[1],
            _ => &self.palettes[0],
        }
    }

    fn read_write_palette_entry(&self) -> Option<PaletteEntry> {
        self.is_supported_palette_selected()
            .then(|| self.read_write_palette()[usize::from(self.palette_index)])
    }

    pub(super) fn palette_index(&self, pixel: u8) -> u8 {
        let offset = self.control & 0x0f;
        (pixel & 0x0f) | ((pixel & 0xf0).wrapping_add(offset << 4) & 0xf0)
    }

    fn is_supported_palette_selected(&self) -> bool {
        matches!((self.palette_control >> 4) & 0x07, 1 | 5 | 2 | 6)
    }

    fn write_palette_mut(&mut self) -> Option<&mut [PaletteEntry; 256]> {
        let palette = (self.palette_control >> 4) & 0x07;
        match palette {
            1 | 5 => Some(&mut self.palettes[usize::from(palette == 5)]),
            2 | 6 => Some(&mut self.sprite_palettes[usize::from(palette == 6)]),
            _ => None,
        }
    }

    fn write_palette_8(&mut self, value: u8) {
        self.palette_write_9bit = false;
        let index = usize::from(self.palette_index);
        if let Some(palette) = self.write_palette_mut() {
            let blue_low = value & 3;
            let blue = blue_low | (((blue_low | (blue_low >> 1)) & 1) << 2);
            palette[index] = PaletteEntry {
                rrr_ggg_bb: value,
                priority_and_blue: (blue >> 2) & 1,
            };
        }
        self.increment_palette_index();
    }

    fn write_palette_9(&mut self, value: u8) {
        let index = usize::from(self.palette_index);
        if self.palette_write_9bit {
            let first_byte = self.palette_pending_first_byte;
            let palette_family = (self.palette_control >> 4) & 0x07;
            let priority = if matches!(palette_family, 1 | 5) {
                value & 0x80
            } else {
                0
            };
            if let Some(palette) = self.write_palette_mut() {
                palette[index] = PaletteEntry {
                    rrr_ggg_bb: first_byte,
                    priority_and_blue: priority | (value & 0x01),
                };
            }
            self.palette_write_9bit = false;
            self.increment_palette_index();
        } else {
            self.palette_pending_first_byte = value;
            self.palette_write_9bit = true;
        }
    }

    fn increment_palette_index(&mut self) {
        if self.palette_control & 0x80 == 0 {
            self.palette_index = self.palette_index.wrapping_add(1);
        }
    }
}
