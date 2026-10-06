//! Frame-based composition of Spectrum Next video layers.

use bus::{NextBus, VideoColor};

const SPRITE_WIDTH: usize = 320;
const SPRITE_HEIGHT: usize = 256;

/// Composite the implemented sprite, Layer 2, and ULA layers into a Next frame.
pub(super) fn compose(bus: &NextBus, output: &mut [u8], with_border: bool) {
    let order = bus.video_layer_order();
    if order >= 6 {
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
            let color = layer2
                .filter(|pixel| pixel.priority)
                .or_else(|| top_layer_color(layer_order, sprite, layer2));
            if let Some(color) = color {
                let output_offset = (y * width + x) * 4;
                output[output_offset..output_offset + 3].copy_from_slice(&color.rgb);
                output[output_offset + 3] = 0xff;
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
) -> Option<VideoColor> {
    for layer in order {
        match layer {
            VideoLayer::Sprite if sprite.is_some() => return sprite,
            VideoLayer::Layer2 if layer2.is_some() => return layer2,
            VideoLayer::Ula => return None,
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

    #[test]
    fn sprites_and_layer2_follow_all_six_supported_layer_orders() {
        let mut machine = test_machine();
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
