//! Optional libretro host. The emulator and format behavior remain in Spec Chum crates.

use std::path::Path;

use libretro_core::{
    export_core, fixed_system_av_info, Core, CoreEventConfig, GameInfo, JoypadButton,
    KeyboardEvent, KeyboardKey, PixelFormat, Runtime, SystemAvInfo, SystemInfo,
};
use spec_chum_host::{HostSession, ModelId, AUDIO_SAMPLE_RATE};

const FRAME_RATE: f64 = 50.0;
const DEFAULT_WIDTH: u32 = 352;
const DEFAULT_HEIGHT: u32 = 296;
const MAX_WIDTH: u32 = 512;
const MAX_HEIGHT: u32 = 312;
const VALID_EXTENSIONS: &str = "tap|tzx|sna|z80|rzx|dsk|trd";

fn joystick_mask(buttons: [bool; 5]) -> u8 {
    buttons
        .into_iter()
        .zip([0x01, 0x02, 0x04, 0x08, 0x10])
        .fold(0, |mask, (pressed, bit)| {
            mask | if pressed { bit } else { 0 }
        })
}

fn rgba_to_xrgb(rgba: &[u8], xrgb: &mut Vec<u8>) {
    xrgb.clear();
    xrgb.reserve(rgba.len());
    for pixel in rgba.as_chunks::<4>().0 {
        let packed = u32::from_be_bytes([0, pixel[0], pixel[1], pixel[2]]);
        xrgb.extend_from_slice(&packed.to_ne_bytes());
    }
}

fn sync_keys(held_keys: &[KeyboardKey]) -> spec_chum_host::keymap::SyncKeys {
    let key_codes: Vec<_> = held_keys.iter().filter_map(|key| key_code(*key)).collect();
    let shift = held_keys
        .iter()
        .any(|key| matches!(key, KeyboardKey::LeftShift | KeyboardKey::RightShift));
    let option = held_keys
        .iter()
        .any(|key| matches!(key, KeyboardKey::LeftAlt | KeyboardKey::RightAlt));
    let control = held_keys
        .iter()
        .any(|key| matches!(key, KeyboardKey::LeftCtrl | KeyboardKey::RightCtrl));
    spec_chum_host::keymap::sync_keys(&key_codes, shift, option, control)
}

fn key_code(key: KeyboardKey) -> Option<u16> {
    Some(match key {
        KeyboardKey::Num1 => 18,
        KeyboardKey::Num2 => 19,
        KeyboardKey::Num3 => 20,
        KeyboardKey::Num4 => 21,
        KeyboardKey::Num5 => 23,
        KeyboardKey::Num6 => 22,
        KeyboardKey::Num7 => 26,
        KeyboardKey::Num8 => 28,
        KeyboardKey::Num9 => 25,
        KeyboardKey::Num0 => 29,
        KeyboardKey::A => 0,
        KeyboardKey::S => 1,
        KeyboardKey::D => 2,
        KeyboardKey::F => 3,
        KeyboardKey::H => 4,
        KeyboardKey::G => 5,
        KeyboardKey::Z => 6,
        KeyboardKey::X => 7,
        KeyboardKey::C => 8,
        KeyboardKey::V => 9,
        KeyboardKey::B => 11,
        KeyboardKey::Q => 12,
        KeyboardKey::W => 13,
        KeyboardKey::E => 14,
        KeyboardKey::R => 15,
        KeyboardKey::Y => 16,
        KeyboardKey::T => 17,
        KeyboardKey::U => 32,
        KeyboardKey::I => 34,
        KeyboardKey::O => 31,
        KeyboardKey::P => 35,
        KeyboardKey::J => 38,
        KeyboardKey::M => 46,
        KeyboardKey::N => 45,
        KeyboardKey::K => 40,
        KeyboardKey::L => 37,
        KeyboardKey::Return => spec_chum_host::keymap::ansi::RETURN,
        KeyboardKey::Space => spec_chum_host::keymap::ansi::SPACE,
        KeyboardKey::Backspace | KeyboardKey::Delete => spec_chum_host::keymap::ansi::DELETE,
        KeyboardKey::Quote => spec_chum_host::keymap::ansi::QUOTE,
        KeyboardKey::Semicolon => spec_chum_host::keymap::ansi::SEMICOLON,
        KeyboardKey::Comma => spec_chum_host::keymap::ansi::COMMA,
        KeyboardKey::Period => spec_chum_host::keymap::ansi::PERIOD,
        KeyboardKey::Slash => spec_chum_host::keymap::ansi::SLASH,
        KeyboardKey::Minus => spec_chum_host::keymap::ansi::MINUS,
        KeyboardKey::Equals => spec_chum_host::keymap::ansi::EQUALS,
        KeyboardKey::LeftBracket => spec_chum_host::keymap::ansi::OPEN_BRACKET,
        KeyboardKey::RightBracket => spec_chum_host::keymap::ansi::CLOSE_BRACKET,
        KeyboardKey::Backslash => spec_chum_host::keymap::ansi::BACKSLASH,
        KeyboardKey::Backquote => spec_chum_host::keymap::ansi::BACKTICK,
        KeyboardKey::Tab => spec_chum_host::keymap::ansi::TAB,
        KeyboardKey::Left => spec_chum_host::keymap::ansi::LEFT,
        KeyboardKey::Right => spec_chum_host::keymap::ansi::RIGHT,
        KeyboardKey::Up => spec_chum_host::keymap::ansi::UP,
        KeyboardKey::Down => spec_chum_host::keymap::ansi::DOWN,
        _ => return None,
    })
}

#[derive(Debug, Default)]
pub struct SpecChumCore {
    session: Option<HostSession>,
    pixels: Vec<u8>,
    audio: Vec<[i16; 2]>,
    held_keys: Vec<KeyboardKey>,
    keyboard_matrix: [[bool; 5]; 8],
    pending_error: Option<String>,
}

impl SpecChumCore {
    fn load_content(&mut self, path: &Path) -> Result<(), String> {
        let extension = path
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        let model = match extension.as_str() {
            "dsk" => ModelId::SpectrumPlus3,
            "trd" => ModelId::Spectrum128,
            _ => ModelId::Spectrum48,
        };
        let mut session = HostSession::new(model, true);
        session
            .select_model(model)
            .map_err(|error| error.to_string())?;

        let result = match extension.as_str() {
            "tap" | "tzx" => session.open_tape(path),
            "sna" | "z80" => session.load_snapshot(path),
            "rzx" => session.load_rzx(path),
            "dsk" => session.load_dsk(path),
            "trd" => session.load_trd(path),
            _ => return Err(format!("unsupported content extension: {extension}")),
        };
        result.map_err(|error| error.to_string())?;
        if session.has_tape() {
            if let Some(machine) = session.machine_mut() {
                machine.type_load_quotes(false);
            }
            session.play_tape().map_err(|error| error.to_string())?;
        }
        session.refresh_framebuffer();
        self.pixels.clear();
        self.audio.clear();
        self.held_keys.clear();
        self.keyboard_matrix = [[false; 5]; 8];
        self.pending_error = None;
        self.session = Some(session);
        Ok(())
    }

    fn keyboard_event(&mut self, event: KeyboardEvent) {
        if event.down {
            if !self.held_keys.contains(&event.key) {
                self.held_keys.push(event.key);
            }
        } else {
            self.held_keys.retain(|&key| key != event.key);
        }
        self.sync_keyboard();
    }

    fn sync_keyboard(&mut self) {
        let synced = sync_keys(&self.held_keys);
        let mut desired = [[false; 5]; 8];
        for (row, bit) in synced.modifiers.into_iter().chain(synced.matrix) {
            desired[row][usize::from(bit)] = true;
        }

        if let Some(session) = self.session.as_mut() {
            for (row, (previous, next)) in self.keyboard_matrix.iter_mut().zip(desired).enumerate()
            {
                for (bit, (was_pressed, is_pressed)) in previous.iter_mut().zip(next).enumerate() {
                    let Ok(bit) = u8::try_from(bit) else {
                        continue;
                    };
                    if *was_pressed != is_pressed && session.set_key(row, bit, is_pressed).is_ok() {
                        *was_pressed = is_pressed;
                    }
                }
            }
        }
    }

    fn keyboard_joystick_mask(&self) -> u8 {
        sync_keys(&self.held_keys).kempston_mask
    }
}

impl Core for SpecChumCore {
    fn configure_events(&mut self, events: &mut CoreEventConfig<Self>) {
        events.add_keyboard_event_listener(Self::keyboard_event);
    }

    fn system_info(&self) -> SystemInfo {
        let mut info = SystemInfo::new("Spec Chum", env!("CARGO_PKG_VERSION"));
        info.valid_extensions = Some(VALID_EXTENSIONS.into());
        info.need_fullpath = true;
        info
    }

    fn av_info(&self) -> SystemAvInfo {
        let (width, height) = self
            .session
            .as_ref()
            .map_or((DEFAULT_WIDTH, DEFAULT_HEIGHT), |session| {
                (session.width() as u32, session.height() as u32)
            });
        let mut info =
            fixed_system_av_info(width, height, FRAME_RATE, f64::from(AUDIO_SAMPLE_RATE));
        info.geometry.max_width = MAX_WIDTH;
        info.geometry.max_height = MAX_HEIGHT;
        info
    }

    fn load_game(&mut self, game: Option<GameInfo<'_>>, runtime: &mut Runtime<'_>) -> bool {
        let Some(path) = game.and_then(|game| game.path_lossy()) else {
            return false;
        };
        if !runtime
            .environment()
            .set_pixel_format(PixelFormat::Xrgb8888)
        {
            return false;
        }
        match self.load_content(Path::new(path.as_ref())) {
            Ok(()) => true,
            Err(error) => {
                runtime.set_message(error, 300);
                false
            }
        }
    }

    fn unload_game(&mut self) {
        self.session = None;
        self.pixels.clear();
        self.audio.clear();
        self.held_keys.clear();
        self.keyboard_matrix = [[false; 5]; 8];
    }

    fn reset(&mut self) {
        if let Some(session) = self.session.as_mut() {
            self.pending_error = session.reset().err().map(|error| error.to_string());
        }
    }

    fn run(&mut self, runtime: &mut Runtime<'_>) {
        runtime.poll_input();
        self.sync_keyboard();
        if let Some(error) = self.pending_error.take() {
            runtime.set_message(error, 300);
        }
        let mask = joystick_mask([
            runtime.joypad_pressed(0, JoypadButton::Right),
            runtime.joypad_pressed(0, JoypadButton::Left),
            runtime.joypad_pressed(0, JoypadButton::Down),
            runtime.joypad_pressed(0, JoypadButton::Up),
            runtime.joypad_pressed(0, JoypadButton::A),
        ]);
        let joystick_mask = mask | self.keyboard_joystick_mask();
        let Some(session) = self.session.as_mut() else {
            runtime.video_refresh_dupe(DEFAULT_WIDTH, DEFAULT_HEIGHT);
            runtime.audio_sample_batch(&[]);
            return;
        };

        if let Err(error) = session.set_joystick(joystick_mask) {
            runtime.set_message(error.to_string(), 300);
        }
        session.run_frame();

        let width = session.width();
        let height = session.height();
        rgba_to_xrgb(session.framebuffer(), &mut self.pixels);
        self.audio.clear();
        self.audio
            .extend(
                session
                    .audio_pcm_stereo()
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|[left, right]| {
                        let quantize =
                            |sample: f32| (sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16;
                        [quantize(*left), quantize(*right)]
                    }),
            );

        runtime.video_refresh_frame(&self.pixels, width as u32, height as u32, width * 4);
        runtime.audio_sample_batch(&self.audio);
    }
}

export_core!(SpecChumCore::default());

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advertises_supported_content_and_path_loading() {
        let info = SpecChumCore::default().system_info();
        assert_eq!(info.valid_extensions.as_deref(), Some(VALID_EXTENSIONS));
        assert!(info.need_fullpath);
    }

    #[test]
    fn reports_spectrum_frame_timing_and_bounds() {
        let info = SpecChumCore::default().av_info();
        assert_eq!(info.geometry.base_width, DEFAULT_WIDTH);
        assert_eq!(info.geometry.base_height, DEFAULT_HEIGHT);
        assert_eq!(info.geometry.max_width, MAX_WIDTH);
        assert_eq!(info.geometry.max_height, MAX_HEIGHT);
        assert_eq!(info.timing.fps, FRAME_RATE);
        assert_eq!(info.timing.sample_rate, f64::from(AUDIO_SAMPLE_RATE));
    }

    #[test]
    fn maps_joypad_directions_and_a_to_kempston() {
        assert_eq!(joystick_mask([true, false, true, false, true]), 0x15);
    }

    #[test]
    fn converts_rgba_bytes_to_xrgb8888_pixels() {
        let mut pixels = Vec::new();
        rgba_to_xrgb(&[0x12, 0x34, 0x56, 0xff], &mut pixels);
        assert_eq!(pixels, 0x0012_3456_u32.to_ne_bytes());
    }

    #[test]
    fn maps_keyboard_through_shared_spectrum_chords() {
        let synced = sync_keys(&[KeyboardKey::LeftShift, KeyboardKey::Num1]);
        let keys: Vec<_> = synced.modifiers.into_iter().chain(synced.matrix).collect();
        assert_eq!(keys, vec![(7, 1), (3, 0)]);
    }

    #[test]
    fn routes_arrow_keys_and_tab_to_kempston() {
        let synced = sync_keys(&[
            KeyboardKey::Right,
            KeyboardKey::Left,
            KeyboardKey::Down,
            KeyboardKey::Up,
            KeyboardKey::Tab,
        ]);
        assert_eq!(synced.matrix.len(), 0);
        assert_eq!(synced.kempston_mask, 0x1f);
    }
}
