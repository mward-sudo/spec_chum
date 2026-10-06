//! Frame-based composition of Spectrum Next video layers.

use bus::{NextBus, VideoColor};

const SPRITE_WIDTH: usize = 320;
const SPRITE_HEIGHT: usize = 256;
const LORES_WIDTH: usize = 128;
const LORES_HEIGHT: usize = 96;
const TILEMAP_WIDTH: usize = 320;
const TILEMAP_HEIGHT: usize = 256;

/// Replace the ULA paper with 256-colour `LoRes` pixels when enabled.
pub(super) fn render_lores(bus: &NextBus, screen: &[u8], output: &mut [u8], with_border: bool) {
    if bus.radastan_lores_enabled() {
        render_radastan(bus, screen, output, with_border);
        return;
    }
    if !bus.lores_256_color_enabled() || screen.len() < 0x3800 {
        return;
    }
    let (width, origin_x, origin_y) = if with_border {
        (352usize, 48usize, 48usize)
    } else {
        (256usize, 0usize, 0usize)
    };

    for y in 0..LORES_HEIGHT {
        let line_offset = if y < 48 {
            y * LORES_WIDTH
        } else {
            0x2000 + (y - 48) * LORES_WIDTH
        };
        for x in 0..LORES_WIDTH {
            let Some(color) = bus.lores_video_pixel(screen[line_offset + x]) else {
                for dy in 0..2 {
                    for dx in 0..2 {
                        let output_offset =
                            (((origin_y + y * 2 + dy) * width) + origin_x + x * 2 + dx) * 4;
                        output[output_offset..output_offset + 4].copy_from_slice(&[0, 0, 0, 0]);
                    }
                }
                continue;
            };
            for dy in 0..2 {
                for dx in 0..2 {
                    let output_offset =
                        (((origin_y + y * 2 + dy) * width) + origin_x + x * 2 + dx) * 4;
                    output[output_offset..output_offset + 3].copy_from_slice(&color.rgb);
                    output[output_offset + 3] = 0xff;
                }
            }
        }
    }
}

fn render_radastan(bus: &NextBus, screen: &[u8], output: &mut [u8], with_border: bool) {
    let file_offset = bus.radastan_display_file_offset();
    if screen.len() < file_offset + 0x1800 {
        return;
    }
    let (width, origin_x, origin_y) = if with_border {
        (352usize, 48usize, 48usize)
    } else {
        (256usize, 0usize, 0usize)
    };

    for y in 0..LORES_HEIGHT {
        let line_offset = file_offset + y * (LORES_WIDTH / 2);
        for x in 0..LORES_WIDTH {
            let packed = screen[line_offset + x / 2];
            let pixel = if x & 1 == 0 {
                packed >> 4
            } else {
                packed & 0x0f
            };
            let output_offset = (((origin_y + y * 2) * width) + origin_x + x * 2) * 4;
            if let Some(color) = bus.radastan_video_pixel(pixel) {
                for row in 0..2 {
                    let offset = output_offset + row * width * 4;
                    output[offset..offset + 3].copy_from_slice(&color.rgb);
                    output[offset + 3] = 0xff;
                    output[offset + 4..offset + 7].copy_from_slice(&color.rgb);
                    output[offset + 7] = 0xff;
                }
            } else {
                for row in 0..2 {
                    let offset = output_offset + row * width * 4;
                    output[offset..offset + 8].fill(0);
                }
            }
        }
    }
}

/// Composite the standard tilemap with ULA before the other video layers.
pub(super) fn render_tilemap(bus: &NextBus, output: &mut [u8], with_border: bool) {
    let (width, height) = if with_border {
        (352usize, 288usize)
    } else {
        (256, 192)
    };
    if output.len() < width * height * 4 {
        return;
    }
    let origin = if with_border {
        (16isize, 16isize)
    } else {
        (-32, -32)
    };

    for y in 0..height {
        for x in 0..width {
            let tile_x = x as isize - origin.0;
            let tile_y = y as isize - origin.1;
            if !(0..TILEMAP_WIDTH as isize).contains(&tile_x)
                || !(0..TILEMAP_HEIGHT as isize).contains(&tile_y)
            {
                continue;
            }
            let output_offset = (y * width + x) * 4;
            if let Some((color, over_ula)) =
                bus.tilemap_video_pixel(tile_x as usize, tile_y as usize)
            {
                if over_ula || output[output_offset + 3] == 0 {
                    output[output_offset..output_offset + 3].copy_from_slice(&color.rgb);
                    output[output_offset + 3] = 0xff;
                }
            }
        }
    }
}

/// Composite the implemented sprite, Layer 2, and ULA layers into a Next frame.
pub(super) fn compose(bus: &NextBus, output: &mut [u8], with_border: bool) {
    let order = bus.video_layer_order();
    if order >= 6 {
        for offset in (0..output.len()).step_by(4) {
            if output[offset + 3] == 0 {
                output[offset..offset + 4].copy_from_slice(&[0, 0, 0, 0xff]);
            }
        }
        return;
    }

    let sprite_surface = bus.render_sprite_surface();
    let (width, height) = if with_border { (352, 288) } else { (256, 192) };
    let sprite_origin = if with_border {
        (16isize, 16isize)
    } else {
        (-32, -32)
    };
    let layer_order = layer_order(order);

    for y in 0..height {
        for x in 0..width {
            let sprite_x = x as isize - sprite_origin.0;
            let sprite_y = y as isize - sprite_origin.1;
            if !(0..SPRITE_WIDTH as isize).contains(&sprite_x)
                || !(0..SPRITE_HEIGHT as isize).contains(&sprite_y)
            {
                continue;
            }

            let sprite_x = sprite_x as usize;
            let sprite_y = sprite_y as usize;
            let sprite = sprite_surface[sprite_y * SPRITE_WIDTH + sprite_x];
            let layer2 = if (32..288).contains(&sprite_x) && (32..224).contains(&sprite_y) {
                bus.layer2_video_pixel(sprite_x - 32, sprite_y - 32)
            } else {
                None
            };
            let output_offset = (y * width + x) * 4;
            let ula_transparent = output[output_offset + 3] == 0;
            let color = layer2
                .filter(|pixel| pixel.priority)
                .or_else(|| top_layer_color(layer_order, sprite, layer2, ula_transparent));
            if let Some(color) = color {
                output[output_offset..output_offset + 3].copy_from_slice(&color.rgb);
                output[output_offset + 3] = 0xff;
            } else if ula_transparent {
                output[output_offset..output_offset + 4].copy_from_slice(&[0, 0, 0, 0xff]);
            }
        }
    }
}

fn layer_order(order: u8) -> [VideoLayer; 3] {
    use VideoLayer::{Layer2, Sprite, Ula};
    match order {
        0 => [Sprite, Layer2, Ula],
        1 => [Layer2, Sprite, Ula],
        2 => [Sprite, Ula, Layer2],
        3 => [Layer2, Ula, Sprite],
        4 => [Ula, Sprite, Layer2],
        _ => [Ula, Layer2, Sprite],
    }
}

#[derive(Clone, Copy)]
enum VideoLayer {
    Sprite,
    Layer2,
    Ula,
}

fn top_layer_color(
    order: [VideoLayer; 3],
    sprite: Option<VideoColor>,
    layer2: Option<VideoColor>,
    ula_transparent: bool,
) -> Option<VideoColor> {
    for layer in order {
        match layer {
            VideoLayer::Sprite if sprite.is_some() => return sprite,
            VideoLayer::Layer2 if layer2.is_some() => return layer2,
            VideoLayer::Ula if !ula_transparent => return None,
            VideoLayer::Ula => {}
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use crate::NextMachine;

    fn test_machine() -> NextMachine {
        NextMachine::new(&vec![0; bus::NEXT_ROM_SIZE]).expect("valid test ROM")
    }

    fn write_sprite(machine: &mut NextMachine, x: u8, y: u8, pixel: u8) {
        machine.bus.out_port(0x303b, 0);
        machine.bus.out_port(0x005b, pixel);
        machine.bus.out_port(0x303b, 0);
        for value in [x, y, 0, 0x80] {
            machine.bus.out_port(0x0057, value);
        }
    }

    fn pixel(frame: &[u8], width: usize, x: usize, y: usize) -> [u8; 4] {
        let offset = (y * width + x) * 4;
        frame[offset..offset + 4]
            .try_into()
            .expect("one RGBA pixel")
    }

    fn setup_tilemap(machine: &mut NextMachine, control: u8) {
        let mut map = vec![0u8; 40 * 32 * 2];
        for attribute in map.iter_mut().skip(1).step_by(2) {
            *attribute = 0x10;
        }
        assert!(machine.bus.load_ram_page(11, 0x0c00, &map));
        let mut tiles = vec![0x11; 32];
        tiles[0] = 0x12;
        assert!(machine.bus.load_ram_page(10, 0x0c00, &tiles));

        machine.bus.write_nextreg(0x43, 0x30); // Tilemap palette 1.
        machine.bus.write_nextreg(0x40, 0x11);
        machine.bus.write_nextreg(0x41, 0xe0); // Red.
        machine.bus.write_nextreg(0x40, 0x12);
        machine.bus.write_nextreg(0x41, 0x1c); // Green.
        machine.bus.write_nextreg(0x43, 0);
        machine.bus.write_nextreg(0x6b, control);
    }

    #[test]
    fn standard_tilemap_decodes_packed_pixels_palette_and_transparency() {
        let mut machine = test_machine();
        setup_tilemap(&mut machine, 0x81);
        machine.bus.write_nextreg(0x4c, 0xf);

        let (width, height) = NextMachine::framebuffer_dims(true);
        let mut frame = vec![0; width * height * 4];
        machine.render_rgba(&mut frame, true);
        assert_eq!(pixel(&frame, width, 16, 16), [255, 0, 0, 255]);
        assert_eq!(pixel(&frame, width, 17, 16), [0, 255, 0, 255]);
        assert_eq!(pixel(&frame, width, 335, 271), [255, 0, 0, 255]);
        assert_eq!(pixel(&frame, width, 336, 16), [0, 0, 0, 255]);

        machine.bus.write_nextreg(0x4c, 2);
        machine.render_rgba(&mut frame, true);
        assert_eq!(pixel(&frame, width, 17, 16), [0, 0, 0, 255]);
        assert_eq!(frame.len(), width * height * 4);
    }

    #[test]
    fn tile_attributes_apply_id_palette_offset_and_rotation() {
        let mut machine = test_machine();
        setup_tilemap(&mut machine, 0x80);
        // Tile 1 uses palette offset 2 and rotates the source clockwise.
        machine.bus.load_ram_page(11, 0x0c00, &[1, 0x22]);
        let mut tile = vec![0x11; 32];
        tile[0] = 0x12;
        tile[27] = 0x13;
        machine.bus.load_ram_page(10, 0x0c20, &tile);
        machine.bus.write_nextreg(0x43, 0x30);
        machine.bus.write_nextreg(0x40, 0x21);
        machine.bus.write_nextreg(0x41, 0xe0);
        machine.bus.write_nextreg(0x40, 0x22);
        machine.bus.write_nextreg(0x41, 0x1c);
        machine.bus.write_nextreg(0x40, 0x13);
        machine.bus.write_nextreg(0x41, 0x03);

        let Some((rotated, _)) = machine.bus.tilemap_video_pixel(7, 1) else {
            panic!("rotated tile pixel should be opaque");
        };
        assert_eq!(rotated.rgb, [0, 255, 0]);
        let Some((offset_color, _)) = machine.bus.tilemap_video_pixel(0, 0) else {
            panic!("tile pixel should be opaque");
        };
        // The tile's palette offset is applied before palette lookup.
        assert_eq!(offset_color.rgb, [255, 0, 0]);

        machine.bus.load_ram_page(11, 0x0c01, &[0x28]); // Horizontal mirror.
        let Some((mirrored_x, _)) = machine.bus.tilemap_video_pixel(6, 0) else {
            panic!("horizontally mirrored tile pixel should be opaque");
        };
        assert_eq!(mirrored_x.rgb, [0, 255, 0]);
        machine.bus.load_ram_page(11, 0x0c01, &[0x24]); // Vertical mirror.
        let Some((mirrored_y, _)) = machine.bus.tilemap_video_pixel(1, 7) else {
            panic!("vertically mirrored tile pixel should be opaque");
        };
        assert_eq!(mirrored_y.rgb, [0, 255, 0]);

        // Hardware mirrors source coordinates, XORing X mirror with rotation before swapping axes.
        machine.bus.load_ram_page(11, 0x0c01, &[0x1a]); // X mirror + rotate.
        let Some((x_mirror_rotation, _)) = machine.bus.tilemap_video_pixel(0, 1) else {
            panic!("combined X mirror and rotation pixel should be opaque");
        };
        assert_eq!(x_mirror_rotation.rgb, [0, 255, 0]);

        machine.bus.load_ram_page(11, 0x0c01, &[0x16]); // Y mirror + rotate.
        let Some((y_mirror_rotation, _)) = machine.bus.tilemap_video_pixel(1, 0) else {
            panic!("combined Y mirror and rotation pixel should be opaque");
        };
        assert_eq!(y_mirror_rotation.rgb, [0, 0, 255]);
    }

    #[test]
    fn tilemap_bank5_addresses_wrap_at_sixteen_kibibytes() {
        let mut machine = test_machine();
        assert!(machine.bus.load_ram_page(11, 0x1f00, &[8, 0x10]));
        // Base $3f00 + tile 8 * 32 wraps from the end of bank 5 to its start.
        assert!(machine.bus.load_ram_page(10, 0, &[0x21]));

        machine.bus.write_nextreg(0x6e, 0x3f);
        machine.bus.write_nextreg(0x6f, 0x3f);
        machine.bus.write_nextreg(0x43, 0x30);
        machine.bus.write_nextreg(0x40, 0x11);
        machine.bus.write_nextreg(0x41, 0xe0);
        machine.bus.write_nextreg(0x40, 0x12);
        machine.bus.write_nextreg(0x41, 0x1c);
        machine.bus.write_nextreg(0x6b, 0x80);

        let Some((color, _)) = machine.bus.tilemap_video_pixel(0, 0) else {
            panic!("wrapped bank 5 tile pixel should be opaque");
        };
        assert_eq!(color.rgb, [0, 255, 0]);
    }

    #[test]
    fn tilemap_reads_bank7_wraps_addresses_and_uses_selected_palette() {
        let mut machine = test_machine();
        let mut map = vec![0u8; 40 * 32 * 2];
        map[..2].copy_from_slice(&[8, 0x10]);
        assert!(machine.bus.load_ram_page(14, 0x1000, &map));
        // Bank 7 is 8 KiB: tile 8 at base $1f00 wraps to offset zero.
        let mut tile = vec![0x11; 32];
        tile[0] = 0x31;
        assert!(machine.bus.load_ram_page(14, 0, &tile));

        machine.bus.write_nextreg(0x6e, 0x90);
        machine.bus.write_nextreg(0x6f, 0xbf);
        machine.bus.write_nextreg(0x43, 0x70); // Write tilemap palette 2.
        machine.bus.write_nextreg(0x40, 0x13);
        machine.bus.write_nextreg(0x41, 0x03); // Blue.
        machine.bus.write_nextreg(0x43, 0);
        machine.bus.write_nextreg(0x6b, 0x90); // Enable; display palette 2.

        let Some((color, _)) = machine.bus.tilemap_video_pixel(0, 0) else {
            panic!("bank 7 tile pixel should be opaque");
        };
        assert_eq!(color.rgb, [0, 0, 255]);
        assert_eq!(machine.bus.read_nextreg(0x6e), 0x90);
        assert_eq!(machine.bus.read_nextreg(0x6f), 0xbf);
    }

    #[test]
    fn tilemap_clip_scroll_origin_and_ula_priority_are_respected() {
        let mut machine = test_machine();
        setup_tilemap(&mut machine, 0x80);
        let (width, height) = NextMachine::framebuffer_dims(true);
        let mut frame = vec![0; width * height * 4];

        machine.bus.write_nextreg(0x1c, 0);
        for value in [1, 1, 0, 0] {
            machine.bus.write_nextreg(0x1b, value);
        }
        assert_eq!(machine.bus.read_nextreg(0x1c), 0);
        machine.render_rgba(&mut frame, true);
        assert_eq!(pixel(&frame, width, 17, 16), [0, 0, 0, 255]);
        assert_eq!(pixel(&frame, width, 18, 16), [255, 0, 0, 255]);
        assert_eq!(pixel(&frame, width, 18, 17), [0, 0, 0, 255]);
        assert_eq!(pixel(&frame, width, 18, 18), [0, 0, 0, 255]);

        // Full clip window, scroll by one pixel, and put tilemap over ULA.
        machine.bus.write_nextreg(0x1c, 0);
        for value in [0, 159, 0, 255] {
            machine.bus.write_nextreg(0x1b, value);
        }
        machine.bus.write_nextreg(0x2f, 0);
        machine.bus.write_nextreg(0x30, 1);
        machine.bus.write_nextreg(0x31, 0);
        machine.bus.write_nextreg(0x6b, 0x81);
        machine.render_rgba(&mut frame, true);
        assert_eq!(pixel(&frame, width, 16, 16), [0, 255, 0, 255]);

        // Attribute bit zero puts this cell under ULA unless $6B.0 forces it over.
        machine.bus.load_ram_page(11, 0x0c01, &[0x11]);
        machine.bus.write_nextreg(0x30, 0);
        machine.bus.write_nextreg(0x6b, 0x80);
        machine.render_rgba(&mut frame, true);
        assert_eq!(pixel(&frame, width, 16, 16), [0, 0, 0, 255]);
        machine.bus.write_nextreg(0x6b, 0x81);
        machine.render_rgba(&mut frame, true);
        assert_eq!(pixel(&frame, width, 16, 16), [255, 0, 0, 255]);

        machine.bus.write_nextreg(0x6b, 0);
        machine.render_rgba(&mut frame, true);
        assert_eq!(pixel(&frame, width, 16, 16), [0, 0, 0, 255]);
        assert_eq!(frame.len(), width * height * 4);

        machine.bus.write_nextreg(0x6b, 0x81);
        let (plain_width, plain_height) = NextMachine::framebuffer_dims(false);
        let mut plain = vec![0; plain_width * plain_height * 4];
        machine.render_rgba(&mut plain, false);
        // Without the border, output origin is tilemap coordinate (32, 32).
        assert_eq!(pixel(&plain, plain_width, 0, 0), [255, 0, 0, 255]);
        assert_eq!(plain.len(), plain_width * plain_height * 4);
    }

    #[test]
    fn tilemap_and_sprite_clip_indices_are_independent() {
        let mut machine = test_machine();
        machine.bus.write_nextreg(0x1b, 12); // Advance tilemap clip index.
        machine.bus.write_nextreg(0x19, 34); // Advance sprite clip index.
        assert_eq!(machine.bus.read_nextreg(0x1c), 0x44);

        machine.bus.write_nextreg(0x1c, 0x08); // Reset tilemap clip index only.
        assert_eq!(machine.bus.read_nextreg(0x1c), 0x04);
        machine.bus.write_nextreg(0x1c, 0x02); // Reset sprite clip index only.
        assert_eq!(machine.bus.read_nextreg(0x1c), 0);
    }

    #[test]
    fn sprites_and_layer2_follow_all_six_supported_layer_orders() {
        let mut machine = test_machine();
        setup_tilemap(&mut machine, 0x81);
        machine.bus.load_ram_page(16, 0, &[0xe0]);
        write_sprite(&mut machine, 32, 32, 0x03);

        let (width, height) = NextMachine::framebuffer_dims(false);
        let mut frame = vec![0; width * height * 4];
        machine.render_rgba(&mut frame, false);
        let ula = pixel(&frame, width, 0, 0);
        machine.bus.write_nextreg(0x69, 0x80);

        for (order, expected) in [
            [0, 0, 255, 255],
            [255, 0, 0, 255],
            [0, 0, 255, 255],
            [255, 0, 0, 255],
            ula,
            ula,
        ]
        .into_iter()
        .enumerate()
        {
            machine.bus.write_nextreg(0x15, ((order as u8) << 2) | 1);
            machine.render_rgba(&mut frame, false);
            assert_eq!(pixel(&frame, width, 0, 0), expected, "layer order {order}");
        }
    }

    #[test]
    fn sprites_map_to_the_documented_surface_with_and_without_border() {
        let mut machine = test_machine();
        write_sprite(&mut machine, 0, 0, 0xe0);
        machine.bus.write_nextreg(0x15, 0x03); // Visible over border.

        let (width, height) = NextMachine::framebuffer_dims(true);
        let mut frame = vec![0; width * height * 4];
        machine.render_rgba(&mut frame, true);
        assert_eq!(pixel(&frame, width, 16, 16), [255, 0, 0, 255]);
        assert_eq!(pixel(&frame, width, 48, 48), [0, 0, 0, 255]);

        machine.bus.write_nextreg(0x15, 0x01); // Restrict sprites to the ULA paper.
        machine.render_rgba(&mut frame, true);
        assert_eq!(pixel(&frame, width, 16, 16), [0, 0, 0, 255]);
        assert_eq!(frame.len(), width * height * 4);
    }

    #[test]
    fn sprite_index_priority_selects_the_top_overlapping_sprite() {
        let mut machine = test_machine();
        machine.bus.out_port(0x303b, 0);
        machine.bus.out_port(0x005b, 0xe0);
        machine.bus.out_port(0x303b, 1);
        machine.bus.out_port(0x005b, 0x03);
        machine.bus.out_port(0x303b, 1);
        for value in [32, 32, 0, 0x81] {
            machine.bus.out_port(0x0057, value);
        }
        machine.bus.out_port(0x303b, 0);
        for value in [32, 32, 0, 0x80] {
            machine.bus.out_port(0x0057, value);
        }

        let (width, height) = NextMachine::framebuffer_dims(false);
        let mut frame = vec![0; width * height * 4];
        machine.bus.write_nextreg(0x15, 0x01); // Sprite 127 on top.
        machine.render_rgba(&mut frame, false);
        assert_eq!(pixel(&frame, width, 0, 0), [0, 0, 255, 255]);

        machine.bus.write_nextreg(0x15, 0x41); // Sprite 0 on top.
        machine.render_rgba(&mut frame, false);
        assert_eq!(pixel(&frame, width, 0, 0), [255, 0, 0, 255]);
        assert_eq!(frame.len(), width * height * 4);
    }
}
