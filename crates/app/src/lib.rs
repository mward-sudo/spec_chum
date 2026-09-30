//! Spec Chum — egui frontend library (testable without a display).

mod display;
mod frontend;
mod keymap;
mod session;
mod theme;
mod window_capture;

pub use frontend::SpecChumApp;
pub use keymap::MAPPING_DOC;

use control_plane::ControlPlane;
#[cfg(test)]
use eframe::egui;
use machine::{AyStereoMode, JoystickMode, JoystickState, Machine, Model, TapeLoadOptions};
#[cfg(test)]
use spec_chum_host::ModelId;
use spec_chum_host::{UiPreferences, UserMachineConfig};

pub use session::{EmulatorSession, HostAccess};

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn machine_build_error_from_rom_read_displays() {
        let err = machine::MachineBuildError::from(machine::RomReadError::TrdosNotApplicable {
            model: "48K".into(),
        });
        assert!(
            err.to_string().contains("does not use a TR-DOS ROM"),
            "got {err}"
        );
    }

    #[test]
    fn quote_maps_to_symbol_p_on_session() {
        let mut session = EmulatorSession::new(Model::Spectrum48, true);
        session.try_autoload_rom();
        if !session.host_mut().has_machine() {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        }
        let mut keys = std::collections::HashSet::new();
        keys.insert(egui::Key::Quote);
        let mods = egui::Modifiers {
            shift: true,
            ..Default::default()
        };
        session.sync_keyboard(&keys, mods, JoystickState::empty());
        let host = &mut *session.host_mut();
        let kb = host.machine_mut().unwrap().keyboard_mut();
        // Symbol (row7 bit1) and P (row5 bit0) active-low
        assert_eq!(kb.rows[7] & (1 << 1), 0);
        assert_eq!(kb.rows[5] & (1 << 0), 0);
        // Caps must not be forced by Shift for punctuation
        assert_ne!(kb.rows[0] & (1 << 0), 0);
    }

    #[test]
    fn shifted_punctuation_keeps_caps_for_a_simultaneous_letter() {
        let mut session = EmulatorSession::new(Model::Spectrum48, true);
        let rom = vec![0; 16 * 1024];
        session
            .host_mut()
            .set_machine(Machine::new_48k(&rom).expect("valid-sized test ROM"));
        let keys = [egui::Key::Quote, egui::Key::A].into_iter().collect();
        let mods = egui::Modifiers {
            shift: true,
            ..Default::default()
        };

        session.sync_keyboard(&keys, mods, JoystickState::empty());

        let rows = session
            .host_mut()
            .machine_mut()
            .unwrap()
            .keyboard_mut()
            .rows;
        assert_eq!(rows[0] & 1, 0, "Caps Shift stays pressed for A");
        assert_eq!(rows[7] & (1 << 1), 0, "Symbol Shift forms the quote chord");
        assert_eq!(rows[5] & 1, 0, "P forms the quote chord");
        assert_eq!(rows[1] & 1, 0, "A remains shifted");
    }

    #[test]
    fn arrow_left_maps_joystick_kempston_and_cursor_mode() {
        let mut session = EmulatorSession::new(Model::Spectrum48, true);
        session.try_autoload_rom();
        if !session.host_mut().has_machine() {
            return;
        }
        let mut keys = std::collections::HashSet::new();
        keys.insert(egui::Key::ArrowLeft);
        session.host_mut().set_joystick_mode(JoystickMode::Kempston);
        session.sync_keyboard(&keys, egui::Modifiers::default(), JoystickState::empty());
        assert!(
            session
                .host_mut()
                .machine_mut()
                .unwrap()
                .kempston_mut()
                .left
        );

        session.host_mut().set_joystick_mode(JoystickMode::Cursor);
        session.sync_keyboard(&keys, egui::Modifiers::default(), JoystickState::empty());
        let host = &mut *session.host_mut();
        let m = host.machine_mut().unwrap();
        let rows = m.keyboard_mut().rows;
        assert_eq!(rows[0] & 1, 0); // Caps
        assert_eq!(rows[3] & (1 << 4), 0); // 5
        assert!(!m.kempston_mut().left);
    }

    #[test]
    fn physical_num1_survives_sinclair_left_joystick_clear() {
        let mut session = EmulatorSession::new(Model::Spectrum48, true);
        session.try_autoload_rom();
        if !session.host_mut().has_machine() {
            return;
        }
        let mut keys = std::collections::HashSet::new();
        keys.insert(egui::Key::Num1);
        session
            .host_mut()
            .set_joystick_mode(JoystickMode::SinclairLeft);
        session.sync_keyboard(&keys, egui::Modifiers::default(), JoystickState::empty());
        let host = &mut *session.host_mut();
        let rows = host.machine_mut().unwrap().keyboard_mut().rows;
        // Num1 = row 3 bit 0 must stay pressed even though Sinclair clears that row first.
        assert_eq!(rows[3] & (1 << 0), 0);
    }

    #[test]
    fn physical_num5_survives_cursor_joystick_clear() {
        let mut session = EmulatorSession::new(Model::Spectrum48, true);
        session.try_autoload_rom();
        if !session.host_mut().has_machine() {
            return;
        }
        let mut keys = std::collections::HashSet::new();
        keys.insert(egui::Key::Num5);
        session.host_mut().set_joystick_mode(JoystickMode::Cursor);
        session.sync_keyboard(&keys, egui::Modifiers::default(), JoystickState::empty());
        let host = &mut *session.host_mut();
        let rows = host.machine_mut().unwrap().keyboard_mut().rows;
        // Num5 = row 3 bit 4 (also used as Cursor left); physical hold must remain.
        assert_eq!(rows[3] & (1 << 4), 0);
    }

    fn synthetic_sna48_bytes() -> Vec<u8> {
        let mut data = vec![0u8; 49179];
        data[26] = 5;
        data[23] = 0x00;
        data[24] = 0x40;
        data[27] = 0x00;
        data[28] = 0x80;
        data[27 + 0x4000] = 0xaa;
        data
    }

    #[test]
    fn load_snapshot48_switches_from_128k() {
        let rom48 = EmulatorSession::workspace_root().join("roms/spec48.rom");
        if !rom48.is_file() {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        }
        let mut session = EmulatorSession::new(Model::Spectrum128, true);
        session.try_autoload_rom();
        if !session.host_mut().has_machine() {
            eprintln!("skip: 128K ROM missing");
            return;
        }
        let path = std::env::temp_dir().join("spec_chum_app_128_to_48.sna");
        std::fs::write(&path, synthetic_sna48_bytes()).expect("write sna");
        session.load_snapshot(&path);
        assert_eq!(session.model(), Model::Spectrum48);
        assert_eq!(
            session.host_mut().machine().map(Machine::model),
            Some(Model::Spectrum48)
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_snapshot48_switches_from_plus3() {
        let rom48 = EmulatorSession::workspace_root().join("roms/spec48.rom");
        if !rom48.is_file() {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        }
        let mut session = EmulatorSession::new(Model::SpectrumPlus3, true);
        session.try_autoload_rom();
        if !session.host_mut().has_machine() {
            eprintln!("skip: +3 ROM missing");
            return;
        }
        let path = std::env::temp_dir().join("spec_chum_app_plus3_to_48.sna");
        std::fs::write(&path, synthetic_sna48_bytes()).expect("write sna");
        session.load_snapshot(&path);
        assert_eq!(session.model(), Model::Spectrum48);
        assert_eq!(
            session.host_mut().machine().map(Machine::model),
            Some(Model::Spectrum48)
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn tzx_standard_inserts_as_paused_tap() {
        let mut session = EmulatorSession::new(Model::Spectrum48, true);
        session.try_autoload_rom();
        if !session.host_mut().has_machine() {
            return;
        }
        let boggit = PathBuf::from("/Users/michael/Downloads/BoggitThe/The Boggit - Side 1.tzx");
        if !boggit.exists() {
            // Fall back to synthesizing via fixture TAP path only
            let tap =
                EmulatorSession::workspace_root().join("tests/fixtures/tape/minimal_code.tap");
            session.load_tap(&tap);
            assert!(!session.host_mut().machine().unwrap().tape_playing());
            session.play_tape();
            assert!(session.host_mut().machine().unwrap().tape_playing());
            return;
        }
        session.load_tzx(&boggit);
        assert!(
            session.host_mut().status().contains("as TAP"),
            "status={}",
            session.host_mut().status()
        );
        assert!(!session.host_mut().machine().unwrap().tape_playing());
        assert!(session.host_mut().machine().unwrap().has_tape());
        session.type_load_quotes();
        assert!(session.tick_key_script());
        // PRESS/GAP timings need more than 40 frames for the full script.
        let mut finished = false;
        for _ in 1..200 {
            if !session.tick_key_script() {
                finished = true;
                break;
            }
        }
        assert!(finished, "script should finish");
    }

    #[test]
    fn type_load_quotes_code_script_finishes() {
        let mut session = EmulatorSession::new(Model::Spectrum48, true);
        session.try_autoload_rom();
        if !session.host_mut().has_machine() {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        }
        session.type_load_quotes_code();
        assert!(session.tick_key_script());
        let mut finished = false;
        for _ in 1..200 {
            if !session.tick_key_script() {
                finished = true;
                break;
            }
        }
        assert!(finished, "CODE script should finish");
    }

    /// +3 Type LOAD must queue keys (not a disk-Loader hint) and flash-load `attr_mark` (#144).
    #[test]
    fn plus3_type_load_code_flash_loads_attr_mark() {
        let mut session = EmulatorSession::new(Model::SpectrumPlus3, true);
        session.try_autoload_rom();
        if !session.host_mut().has_machine() {
            eprintln!("skip: plus3/plus2a ROM missing");
            return;
        }
        for _ in 0..250 {
            session.tick_frame();
        }
        let tap = EmulatorSession::workspace_root().join("tests/fixtures/tape/attr_mark.tap");
        session.load_tap(&tap);
        session.type_load_quotes_code();
        assert!(
            !session
                .host_mut()
                .status()
                .to_lowercase()
                .contains("tape loader"),
            "must not point +3 users at disk Loader; status={}",
            session.host_mut().status()
        );
        // Menu → 48 BASIC wait (~120) + LOAD "" CODE (~100) needs >300 frames.
        let mut started = false;
        let mut finished = false;
        for _ in 0..500 {
            let active = session.tick_key_script();
            started |= active;
            session.tick_frame();
            if !active && started {
                finished = true;
                break;
            }
        }
        assert!(started && finished, "plus3 Type LOAD script should finish");
        // Instant-style flash for this regression (Play alone is EAR-only).
        if let Some(m) = session.host_mut().machine_mut() {
            let mut opts = m.tape_load_options();
            opts.flash_load = true;
            m.set_tape_load_options(opts);
            m.set_tape_playing(true);
        }
        let mut loaded = false;
        for _ in 0..400 {
            session.tick_frame();
            let host = &*session.host_mut();
            let m = host.machine().unwrap();
            let code_ok = m.read_mem(0x8000) == 0x21
                && m.read_mem(0x8001) == 0x00
                && m.read_mem(0x8002) == 0x58
                && m.read_mem(0x8003) == 0x36
                && m.read_mem(0x8004) == 0xd7
                && m.read_mem(0x8005) == 0xc9;
            if code_ok {
                loaded = true;
                break;
            }
        }
        {
            let host = &*session.host_mut();
            let m = host.machine().unwrap();
            assert!(
                loaded,
                "plus3 egui Type LOAD CODE + flash Play should load attr_mark at 0x8000 (PC={:04X} block={:?})",
                m.cpu().regs.pc,
                m.tape_block()
            );
        }
    }

    #[test]
    fn play_tape_advances_ear_on_fixture() {
        let mut session = EmulatorSession::new(Model::Spectrum48, true);
        session.try_autoload_rom();
        if !session.host_mut().has_machine() {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        }
        let tap = EmulatorSession::workspace_root().join("tests/fixtures/tape/minimal_code.tap");
        session.load_tap(&tap);
        if let Some(m) = session.host_mut().machine_mut() {
            let mut opts = m.tape_load_options();
            opts.flash_load = false;
            m.set_tape_load_options(opts);
        }
        assert!(!session.host_mut().machine().unwrap().tape_playing());
        for _ in 0..3 {
            session.tick_frame();
        }
        assert!(
            !session.host_mut().machine().unwrap().ear(),
            "paused tape must not drive EAR"
        );
        session.play_tape();
        assert!(session.host_mut().machine().unwrap().tape_playing());
        assert!(session.host_mut().status().contains("playing"));
        let mut saw_high = false;
        for _ in 0..8 {
            session.tick_frame();
            if session.host_mut().machine().unwrap().ear() {
                saw_high = true;
                break;
            }
        }
        assert!(saw_high, "Tape → Play must raise EAR during pilot");
    }

    #[test]
    fn session_loads_tap_fixture_headless() {
        let mut session = EmulatorSession::new(Model::Spectrum48, true);
        session.try_autoload_rom();
        if !session.host_mut().has_machine() {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        }
        let tap = EmulatorSession::workspace_root().join("tests/fixtures/tape/minimal_code.tap");
        session.load_tap(&tap);
        assert!(
            session.host_mut().status().contains("Inserted TAP"),
            "status={}",
            session.host_mut().status()
        );
        assert_eq!(
            session.host_mut().machine().and_then(Machine::tape_block),
            Some(0)
        );
        let before = session.host_mut().framebuffer().to_vec();
        for _ in 0..5 {
            session.tick_frame();
        }
        assert_ne!(
            session.host_mut().framebuffer(),
            before.as_slice(),
            "framebuffer should update"
        );
        assert!(session.host_mut().machine().unwrap().cpu().t > 0);
    }

    #[test]
    fn egui_menu_smoke_without_window() {
        let mut app = SpecChumApp::new_with_audio(false);
        let ctx = egui::Context::default();
        let raw = egui::RawInput::default();
        let mut saw_file = false;
        let mut saw_machine = false;
        let _ = ctx.run(raw, |ctx| {
            app.ui(ctx);
            saw_file = true;
            saw_machine = app.session.host_mut().status().contains("Loaded")
                || app.session.host_mut().status().contains("Missing")
                || app.session.host_mut().status().contains("ROM");
        });
        assert!(saw_file);
        assert!(saw_machine, "status={}", app.session.host_mut().status());
        // Second frame after menus exist — menu strip must reserve clickable height.
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            app.ui(ctx);
            assert!(
                theme::menu_bar_min_height() >= 28.0,
                "menu bar must be tall enough to click"
            );
        });
        assert_eq!(app.session.host_mut().framebuffer().len(), 352 * 296 * 4);
        assert_eq!(ctx.style().visuals.panel_fill.a(), 255);
    }

    #[test]
    fn debug_window_smoke_headless() {
        let mut app = SpecChumApp::new_with_audio(false);
        app.session.debug_open = true;
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            app.ui(ctx);
        });
        assert!(app.session.debug_open);
        {
            let host = &mut *app.session.host_mut();
            if let Some(m) = host.machine_mut() {
                let pc = m.cpu().regs.pc;
                m.step_once();
                assert_ne!(m.cpu().regs.pc, pc, "step_once should advance PC");
            }
        }
    }

    #[test]
    fn prefs_restore_model_tape_volume_on_launch() {
        use spec_chum_host::{save_prefs, PrefJoystick, PrefModel, UiPreferences};
        use std::time::{SystemTime, UNIX_EPOCH};

        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let path = std::env::temp_dir().join(format!("spec-chum-app-prefs-{nanos}.json"));
        let prefs = UiPreferences {
            model: PrefModel::Spectrum128,
            tape_experience: true,
            volume: 0.55,
            muted: true,
            throttle: false,
            kempston_mouse: true,
            joystick_mode: PrefJoystick::Cursor,
            ..UiPreferences::default()
        };
        save_prefs(&path, &prefs).expect("save prefs");

        let mut app = SpecChumApp::new_with_audio_prefs(false, path.clone());
        let _ = std::fs::remove_file(&path);

        assert_eq!(app.session.model(), Model::Spectrum128);
        assert!((app.session.volume - 0.55).abs() < f32::EPSILON);
        assert!(app.session.muted);
        assert!(!app.session.throttle);
        assert!(app.session.kempston_mouse);
        assert_eq!(app.session.host_mut().joystick_mode(), JoystickMode::Cursor);
        {
            let host = &*app.session.host_mut();
            if let Some(m) = host.machine() {
                let opts = m.tape_load_options();
                assert!(opts.experience_load);
                assert!(!opts.flash_load);
            }
        }
    }

    #[test]
    fn emulator_session_uses_host_session() {
        let mut session = EmulatorSession::new(Model::Spectrum48, true);
        assert_eq!(session.host_mut().model(), ModelId::Spectrum48);
        assert!(session.host_mut().running());
        let _ = session.host_mut().inspect_text(); // HostSession debug API available
    }

    #[test]
    fn gui_and_control_plane_share_live_session() {
        let mut session = EmulatorSession::new(Model::Spectrum48, true);
        session.try_autoload_rom();
        if !session.host_mut().has_machine() {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        }
        let plane = ControlPlane::from_shared(session.share_host());
        session.host_mut().set_running(false);
        let after = {
            let host = &mut *session.host_mut();
            let m = host.machine_mut().expect("machine");
            let before = m.cpu().regs.pc;
            m.step_once();
            let after = m.cpu().regs.pc;
            assert_ne!(after, before, "GUI step should advance PC");
            after
        };
        let inspect = plane.inspect_json().expect("inspect");
        assert!(
            inspect.contains(&format!("\"pc\":{after}"))
                || inspect.contains(&format!("\"pc\": {after}")),
            "control_plane inspect must see GUI PC={after}: {inspect}"
        );
        plane.with_host_mut(|host| {
            if let Some(m) = host.machine_mut() {
                m.step_once();
            }
        });
        let gui_pc = {
            let host = &*session.host_mut();
            host.machine().map(|m| m.cpu().regs.pc).expect("machine")
        };
        let plane_pc =
            plane.with_host_ref(|host| host.machine().map(|m| m.cpu().regs.pc).expect("machine"));
        assert_eq!(
            gui_pc, plane_pc,
            "GUI and ControlPlane must share one machine"
        );
    }
}
