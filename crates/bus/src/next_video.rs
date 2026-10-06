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
    lores_control: u8,
    timex_video_control: u8,
    transparency: u8,
    tilemap_control: u8,
    tilemap_base: u8,
    tile_base: u8,
    tilemap_transparency: u8,
    tilemap_x_offset: u16,
    tilemap_y_offset: u8,
    tilemap_clip: [u8; 4],
    tilemap_clip_index: u8,
    palette_control: u8,
    palette_index: u8,
    palette_write_9bit: bool,
    palette_pending_first_byte: u8,
    ula_palettes: [[PaletteEntry; 256]; 2],
    layer2_palettes: [[PaletteEntry; 256]; 2],
    sprite_palettes: [[PaletteEntry; 256]; 2],
    tilemap_palettes: [[PaletteEntry; 256]; 2],
}

impl NextVideo {
    pub(super) fn new() -> Self {
        Self {
            first_bank: 8,
            control: 0,
            visible: false,
            priority: 0,
            lores_control: 0,
            timex_video_control: 0,
            transparency: 0xe3,
            tilemap_control: 0,
            tilemap_base: 0x2c,
            tile_base: 0x0c,
            tilemap_transparency: 0x0f,
            tilemap_x_offset: 0,
            tilemap_y_offset: 0,
            tilemap_clip: [0, 159, 0, 255],
            tilemap_clip_index: 0,
            palette_control: 0,
            palette_index: 0,
            palette_write_9bit: false,
            palette_pending_first_byte: 0,
            ula_palettes: [PaletteEntry::default_rgb332_palette(); 2],
            layer2_palettes: [PaletteEntry::default_rgb332_palette(); 2],
            sprite_palettes: [PaletteEntry::default_rgb332_palette(); 2],
            tilemap_palettes: [PaletteEntry::default_rgb332_palette(); 2],
        }
    }

    pub(super) fn reset(&mut self) {
        *self = Self::new();
    }

    pub(super) fn reset_preserving_palette(&mut self) {
        let ula_palettes = self.ula_palettes;
        let layer2_palettes = self.layer2_palettes;
        let sprite_palettes = self.sprite_palettes;
        let tilemap_palettes = self.tilemap_palettes;
        self.reset();
        self.ula_palettes = ula_palettes;
        self.layer2_palettes = layer2_palettes;
        self.sprite_palettes = sprite_palettes;
        self.tilemap_palettes = tilemap_palettes;
    }

    pub(super) fn clip_index(&self) -> u8 {
        self.tilemap_clip_index
    }

    pub(super) fn read_register(&self, register: u8) -> Option<u8> {
        match register {
            0x12 => Some(self.first_bank),
            0x14 => Some(self.transparency),
            0x15 => Some(self.priority),
            0x1b => Some(self.tilemap_clip[usize::from(self.tilemap_clip_index)]),
            0x2f => Some((self.tilemap_x_offset >> 8) as u8),
            0x30 => Some(self.tilemap_x_offset as u8),
            0x31 => Some(self.tilemap_y_offset),
            0x4c => Some(self.tilemap_transparency),
            0x6a => Some(self.lores_control),
            0x6b => Some(self.tilemap_control),
            0x6e => Some(self.tilemap_base),
            0x6f => Some(self.tile_base),
            0x69 => Some((u8::from(self.visible) << 7) | self.timex_video_control),
            0x70 => Some(self.control),
            _ => None,
        }
    }

    pub(super) fn write_register(&mut self, register: u8, value: u8) -> bool {
        match register {
            0x12 => self.first_bank = value & 0x7f,
            0x14 => self.transparency = value,
            0x15 => self.priority = value,
            0x1b => {
                self.tilemap_clip[usize::from(self.tilemap_clip_index)] = value;
                self.tilemap_clip_index = self.tilemap_clip_index.wrapping_add(1) & 3;
            }
            0x1c => {
                if value & 0x08 != 0 {
                    self.tilemap_clip_index = 0;
                }
            }
            0x2f => {
                self.tilemap_x_offset =
                    (u16::from(value & 3) << 8) | (self.tilemap_x_offset & 0xff);
            }
            0x30 => self.tilemap_x_offset = (self.tilemap_x_offset & 0x300) | u16::from(value),
            0x31 => self.tilemap_y_offset = value,
            0x4c => self.tilemap_transparency = value & 0x0f,
            0x6a => self.lores_control = value & 0x3f,
            0x6b => self.tilemap_control = value & !0x04,
            0x6e => self.tilemap_base = value & 0xbf,
            0x6f => self.tile_base = value & 0xbf,
            0x69 => {
                self.visible = value & 0x80 != 0;
                self.write_timex_video_port(value);
            }
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
        let entry = self.active_layer2_palette()[usize::from(index)];
        VideoColor {
            rgb: entry.rgb(),
            priority: entry.priority(),
            transparent: entry.transparent(self.transparency),
        }
    }

    pub(super) fn lores_color(&self, index: u8) -> VideoColor {
        let entry = self.active_ula_palette()[usize::from(index)];
        VideoColor {
            rgb: entry.rgb(),
            priority: false,
            transparent: entry.transparent(self.transparency),
        }
    }

    pub(super) fn radastan_color(&self, pixel: u8) -> VideoColor {
        let index = ((self.lores_control & 0x0f) << 4) | (pixel & 0x0f);
        self.lores_color(index)
    }

    pub(super) fn tilemap_enabled(&self) -> bool {
        self.tilemap_control & 0x80 != 0 && self.tilemap_control & 0x6e == 0
    }

    /// Return a tilemap pixel and whether its cell places it above ULA.
    pub(super) fn tilemap_pixel(
        &self,
        ram: &[u8],
        x: usize,
        y: usize,
    ) -> Option<(VideoColor, bool)> {
        if !self.tilemap_enabled() || x >= 320 || y >= 256 {
            return None;
        }
        let clip_x = x / 2;
        let clip_left = usize::from(self.tilemap_clip[0]);
        let clip_right = usize::from(self.tilemap_clip[1]);
        let clip_top = usize::from(self.tilemap_clip[2]);
        let clip_bottom = usize::from(self.tilemap_clip[3]);
        if clip_x < clip_left || clip_x > clip_right || y < clip_top || y > clip_bottom {
            return None;
        }

        let map_x = (x + usize::from(self.tilemap_x_offset)) % 320;
        let map_y = (y + usize::from(self.tilemap_y_offset)) % 256;
        let tile_x = map_x / 8;
        let tile_y = map_y / 8;
        let entry = tile_y * 40 + tile_x;
        let map_offset = self.tilemap_base_offset() + entry * 2;
        let tile = Self::tilemap_byte(ram, self.tilemap_base, map_offset)?;
        let attribute = Self::tilemap_byte(ram, self.tilemap_base, map_offset + 1)?;

        let rotate = attribute & 0x02 != 0;
        let mut pixel_x = map_x & 7;
        let mut pixel_y = map_y & 7;
        if (attribute & 0x08 != 0) ^ rotate {
            pixel_x = 7 - pixel_x;
        }
        if attribute & 0x04 != 0 {
            pixel_y = 7 - pixel_y;
        }
        if rotate {
            (pixel_x, pixel_y) = (pixel_y, pixel_x);
        }
        let graphics_offset =
            self.tile_base_offset() + usize::from(tile) * 32 + pixel_y * 4 + pixel_x / 2;
        let packed = Self::tilemap_byte(ram, self.tile_base, graphics_offset)?;
        let pixel = if pixel_x & 1 == 0 {
            packed >> 4
        } else {
            packed & 0x0f
        };
        if pixel == self.tilemap_transparency {
            return None;
        }
        let palette = usize::from(self.tilemap_control & 0x10 != 0);
        let palette_index = (attribute & 0xf0) | pixel;
        let entry = self.tilemap_palettes[palette][usize::from(palette_index)];
        let force_over_ula = self.tilemap_control & 1 != 0;
        let ula_over_tilemap = !force_over_ula && attribute & 1 != 0;
        Some((
            VideoColor {
                rgb: entry.rgb(),
                priority: false,
                transparent: false,
            },
            !ula_over_tilemap,
        ))
    }

    fn tilemap_base_offset(&self) -> usize {
        usize::from(self.tilemap_base & 0x3f) << 8
    }

    fn tile_base_offset(&self) -> usize {
        usize::from(self.tile_base & 0x3f) << 8
    }

    fn tilemap_byte(ram: &[u8], base: u8, offset: usize) -> Option<u8> {
        let bank7 = base & 0x80 != 0;
        let bank_size = if bank7 { 0x2000 } else { BANK_SIZE };
        let offset = offset % bank_size;
        let page = if bank7 { 14 } else { 10 } + offset / PAGE_SIZE;
        ram.get(page * PAGE_SIZE + offset % PAGE_SIZE).copied()
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
            0x41 => Some(self.read_write_palette_entry().rrr_ggg_bb),
            0x43 => Some(self.palette_control),
            0x44 => Some(self.read_write_palette_entry().priority_and_blue),
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

    pub(super) fn lores_256_color_mode(&self) -> bool {
        self.lores_control & 0x20 == 0
    }

    pub(super) fn radastan_mode(&self) -> bool {
        self.lores_control & 0x20 != 0
    }

    pub(super) fn write_timex_video_port(&mut self, value: u8) {
        self.timex_video_control = value & 0x3f;
    }

    pub(super) fn radastan_display_file_offset(&self) -> usize {
        let register_file = self.lores_control & 0x10 != 0;
        let timex_file = self.timex_video_control & 1 != 0;
        usize::from(register_file ^ timex_file) * 0x2000
    }

    fn active_ula_palette(&self) -> &[PaletteEntry; 256] {
        &self.ula_palettes[usize::from(self.palette_control & 0x02 != 0)]
    }

    fn active_layer2_palette(&self) -> &[PaletteEntry; 256] {
        &self.layer2_palettes[usize::from(self.palette_control & 0x04 != 0)]
    }

    fn active_sprite_palette(&self) -> &[PaletteEntry; 256] {
        &self.sprite_palettes[usize::from(self.palette_control & 0x08 != 0)]
    }

    fn read_write_palette(&self) -> &[PaletteEntry; 256] {
        match (self.palette_control >> 4) & 0x07 {
            0 => &self.ula_palettes[0],
            4 => &self.ula_palettes[1],
            1 => &self.layer2_palettes[0],
            5 => &self.layer2_palettes[1],
            2 => &self.sprite_palettes[0],
            6 => &self.sprite_palettes[1],
            3 => &self.tilemap_palettes[0],
            7 => &self.tilemap_palettes[1],
            _ => &self.ula_palettes[0],
        }
    }

    fn read_write_palette_entry(&self) -> PaletteEntry {
        self.read_write_palette()[usize::from(self.palette_index)]
    }

    pub(super) fn palette_index(&self, pixel: u8) -> u8 {
        let offset = self.control & 0x0f;
        (pixel & 0x0f) | ((pixel & 0xf0).wrapping_add(offset << 4) & 0xf0)
    }

    fn write_palette_mut(&mut self) -> Option<&mut [PaletteEntry; 256]> {
        let palette = (self.palette_control >> 4) & 0x07;
        match palette {
            0 | 4 => Some(&mut self.ula_palettes[usize::from(palette == 4)]),
            1 | 5 => Some(&mut self.layer2_palettes[usize::from(palette == 5)]),
            2 | 6 => Some(&mut self.sprite_palettes[usize::from(palette == 6)]),
            3 | 7 => Some(&mut self.tilemap_palettes[usize::from(palette == 7)]),
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
