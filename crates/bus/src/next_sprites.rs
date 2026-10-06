//! Spectrum Next's base 8-bit sprite attributes, pattern memory, and ports.

use crate::next_video::{NextVideo, VideoColor};

const SPRITE_COUNT: usize = 128;
const SPRITE_WIDTH: usize = 16;
const SPRITE_HEIGHT: usize = 16;
const PATTERN_BYTES: usize = SPRITE_WIDTH * SPRITE_HEIGHT;
const PATTERN_MEMORY_BYTES: usize = 64 * PATTERN_BYTES;
const SURFACE_WIDTH: usize = 320;
const SURFACE_HEIGHT: usize = 256;
const CLIP_X1: usize = 0;
const CLIP_X2: usize = 255;
const CLIP_Y1: usize = 0;
const CLIP_Y2: usize = 191;

#[derive(Clone, Debug)]
pub(super) struct NextSprites {
    attributes: [[u8; 5]; SPRITE_COUNT],
    pattern_memory: Box<[u8; PATTERN_MEMORY_BYTES]>,
    port_sprite_index: u8,
    port_attribute_index: u8,
    port_pattern_address: usize,
    register_sprite_index: u8,
    transparency: u8,
    clip_window: [u8; 4],
    clip_index: u8,
}

impl NextSprites {
    pub(super) fn new() -> Self {
        Self {
            attributes: [[0; 5]; SPRITE_COUNT],
            pattern_memory: Box::new([0; PATTERN_MEMORY_BYTES]),
            port_sprite_index: 0,
            port_attribute_index: 0,
            port_pattern_address: 0,
            register_sprite_index: 0,
            transparency: 0xe3,
            clip_window: [CLIP_X1 as u8, CLIP_X2 as u8, CLIP_Y1 as u8, CLIP_Y2 as u8],
            clip_index: 0,
        }
    }

    pub(super) fn reset(&mut self) {
        *self = Self::new();
    }

    pub(super) fn reset_preserving_memory(&mut self) {
        let attributes = self.attributes;
        let pattern_memory = self.pattern_memory.clone();
        self.reset();
        self.attributes = attributes;
        self.pattern_memory = pattern_memory;
    }

    pub(super) fn read_register(&self, register: u8) -> Option<u8> {
        match register {
            0x19 => Some(self.clip_window[usize::from(self.clip_index)]),
            0x34 => Some(self.register_sprite_index),
            0x4b => Some(self.transparency),
            _ => None,
        }
    }

    pub(super) fn write_register(
        &mut self,
        register: u8,
        value: u8,
        sprite_index_lockstep: bool,
    ) -> bool {
        match register {
            0x19 => {
                self.clip_window[usize::from(self.clip_index)] = value;
                self.clip_index = (self.clip_index + 1) & 3;
            }
            0x1c => {
                if value & 0x02 != 0 {
                    self.clip_index = 0;
                }
            }
            0x34 => {
                if sprite_index_lockstep {
                    self.select_port(value, true);
                } else {
                    self.register_sprite_index = value & 0x7f;
                }
            }
            0x35..=0x38 => {
                self.write_attribute(self.register_sprite_index, register - 0x35, value);
            }
            0x39 => {
                self.write_attribute(self.register_sprite_index, 4, value);
            }
            0x4b => self.transparency = value,
            0x75..=0x78 => {
                self.write_attribute(self.register_sprite_index, register - 0x75, value);
                self.increment_register_sprite_index(sprite_index_lockstep);
            }
            0x79 => {
                self.write_attribute(self.register_sprite_index, 4, value);
                self.increment_register_sprite_index(sprite_index_lockstep);
            }
            _ => return false,
        }
        true
    }

    pub(super) fn select_port(&mut self, value: u8, sprite_index_lockstep: bool) {
        self.port_sprite_index = value & 0x7f;
        self.port_attribute_index = 0;
        self.port_pattern_address = usize::from(value & 0x3f) * PATTERN_BYTES
            + if value & 0x80 != 0 {
                PATTERN_BYTES / 2
            } else {
                0
            };
        if sprite_index_lockstep {
            self.register_sprite_index = self.port_sprite_index;
        }
    }

    pub(super) fn write_attribute_port(&mut self, value: u8, sprite_index_lockstep: bool) {
        let byte_index = self.port_attribute_index;
        self.write_attribute(self.port_sprite_index, byte_index, value);
        let has_extended_attribute =
            self.attributes[usize::from(self.port_sprite_index)][3] & 0x40 != 0;
        if byte_index == 4 || (byte_index == 3 && !has_extended_attribute) {
            self.port_sprite_index = self.port_sprite_index.wrapping_add(1) & 0x7f;
            if sprite_index_lockstep {
                self.register_sprite_index = self.port_sprite_index;
            }
            self.port_attribute_index = 0;
        } else {
            self.port_attribute_index += 1;
        }
    }

    pub(super) fn write_pattern_port(&mut self, value: u8) {
        self.pattern_memory[self.port_pattern_address] = value;
        self.port_pattern_address = (self.port_pattern_address + 1) % PATTERN_MEMORY_BYTES;
    }

    /// Render the supported sprite features to the hardware's 320×256 surface.
    pub(super) fn render_surface(
        &self,
        video: &NextVideo,
        layer_control: u8,
    ) -> Vec<Option<VideoColor>> {
        let mut surface = vec![None; SURFACE_WIDTH * SURFACE_HEIGHT];
        if layer_control & 0x01 == 0 {
            return surface;
        }

        if layer_control & 0x40 == 0 {
            for sprite_index in 0..SPRITE_COUNT {
                self.draw_sprite(&mut surface, video, layer_control, sprite_index);
            }
        } else {
            for sprite_index in (0..SPRITE_COUNT).rev() {
                self.draw_sprite(&mut surface, video, layer_control, sprite_index);
            }
        }
        surface
    }

    fn draw_sprite(
        &self,
        surface: &mut [Option<VideoColor>],
        video: &NextVideo,
        layer_control: u8,
        sprite_index: usize,
    ) {
        let attributes = self.attributes[sprite_index];
        if attributes[3] & 0x80 == 0 || attributes[2] & 0x0e != 0 {
            return;
        }
        let extended = attributes[3] & 0x40 != 0;
        if extended && attributes[4] & 0xfe != 0 {
            return;
        }
        let y_high_bit = if extended { attributes[4] & 1 } else { 0 };

        let x = usize::from(attributes[0]) | (usize::from(attributes[2] & 0x01) << 8);
        let y = usize::from(attributes[1]) | (usize::from(y_high_bit) << 8);
        let pattern = usize::from(attributes[3] & 0x3f) * PATTERN_BYTES;
        let palette_offset = attributes[2] >> 4;
        let over_border = layer_control & 0x02 != 0;
        let clip_over_border = layer_control & 0x20 != 0;
        for row in 0..SPRITE_HEIGHT {
            let screen_y = y.wrapping_add(row) & 0x1ff;
            if screen_y >= SURFACE_HEIGHT
                || !self.visible_y(screen_y, over_border, clip_over_border)
            {
                continue;
            }
            for column in 0..SPRITE_WIDTH {
                let screen_x = x.wrapping_add(column) & 0x1ff;
                if screen_x >= SURFACE_WIDTH
                    || (!over_border && !in_paper(screen_x, screen_y))
                    || !self.visible_x(screen_x, over_border, clip_over_border)
                {
                    continue;
                }
                let pixel = self.pattern_memory[pattern + row * SPRITE_WIDTH + column];
                let color = video.sprite_color(pixel, palette_offset, self.transparency);
                if !color.transparent {
                    surface[screen_y * SURFACE_WIDTH + screen_x] = Some(color);
                }
            }
        }
    }

    fn visible_x(&self, x: usize, over_border: bool, clip_over_border: bool) -> bool {
        if over_border && !clip_over_border {
            return true;
        }
        let x = if over_border { x } else { x.saturating_sub(32) };
        let (min, max) = if over_border {
            (
                usize::from(self.clip_window[0]) * 2,
                usize::from(self.clip_window[1]) * 2 + 1,
            )
        } else {
            (
                usize::from(self.clip_window[0]),
                usize::from(self.clip_window[1]),
            )
        };
        (min..=max).contains(&x)
    }

    fn visible_y(&self, y: usize, over_border: bool, clip_over_border: bool) -> bool {
        if over_border && !clip_over_border {
            return true;
        }
        let y = if over_border { y } else { y.saturating_sub(32) };
        (usize::from(self.clip_window[2])..=usize::from(self.clip_window[3])).contains(&y)
    }

    fn write_attribute(&mut self, sprite_index: u8, byte_index: u8, value: u8) {
        self.attributes[usize::from(sprite_index)][usize::from(byte_index)] = value;
    }

    fn increment_register_sprite_index(&mut self, sprite_index_lockstep: bool) {
        self.register_sprite_index = self.register_sprite_index.wrapping_add(1) & 0x7f;
        if sprite_index_lockstep {
            self.port_sprite_index = self.register_sprite_index;
            self.port_attribute_index = 0;
        }
    }
}

fn in_paper(x: usize, y: usize) -> bool {
    (32..288).contains(&x) && (32..224).contains(&y)
}
