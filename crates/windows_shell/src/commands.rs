//! Menu command IDs and pure helpers for the Win32 shell (#351 deepen).
//!
//! Shared so keymap-style unit tests can run on non-Windows hosts.

use spec_chum_host::{ModelId, PrefJoystick, PrefModel};

/// File menu
pub const IDM_FILE_OPEN_TAPE: usize = 1001;
pub const IDM_FILE_OPEN_SNAPSHOT: usize = 1002;
pub const IDM_FILE_OPEN_RZX: usize = 1003;
pub const IDM_FILE_OPEN_DSK: usize = 1004;
pub const IDM_FILE_OPEN_TRD: usize = 1005;
pub const IDM_FILE_EXIT: usize = 1009;

/// Tape transport
pub const IDM_TAPE_PLAY: usize = 1101;
pub const IDM_TAPE_PAUSE: usize = 1102;
pub const IDM_TAPE_REWIND: usize = 1103;

/// Machine → model (1200 + ordinal in [`ModelId::ALL`]) and reset
pub const IDM_MACHINE_MODEL_BASE: usize = 1200;
pub const IDM_MACHINE_RESET: usize = 1290;

/// Hardware attach / media
pub const IDM_HW_ATTACH_MULTIFACE: usize = 1301;
pub const IDM_HW_MULTIFACE_NMI: usize = 1302;
pub const IDM_HW_ATTACH_DIVMMC: usize = 1310;
pub const IDM_HW_DIVMMC_SD: usize = 1311;
pub const IDM_HW_DIVMMC_SD_SLOT1: usize = 1312;
pub const IDM_HW_DIVMMC_EEPROM: usize = 1313;
pub const IDM_HW_ATTACH_IF1: usize = 1320;
pub const IDM_HW_INSERT_MDR: usize = 1321;
pub const IDM_HW_ATTACH_BETA: usize = 1330;
pub const IDM_HW_LOAD_TRDOS_ROM: usize = 1331;
pub const IDM_HW_OPEN_TRD: usize = 1332;
pub const IDM_HW_INSERT_DCK: usize = 1340;
pub const IDM_HW_EJECT_DCK: usize = 1341;

/// Settings / prefs
pub const IDM_SET_JOY_KEMPSTON: usize = 1401;
pub const IDM_SET_JOY_SINCLAIR_L: usize = 1402;
pub const IDM_SET_JOY_SINCLAIR_R: usize = 1403;
pub const IDM_SET_JOY_CURSOR: usize = 1404;
pub const IDM_SET_TAPE_EAR_1: usize = 1410;
pub const IDM_SET_TAPE_EAR_2: usize = 1411;
pub const IDM_SET_TAPE_EAR_5: usize = 1412;
pub const IDM_SET_TAPE_EAR_10: usize = 1413;
pub const IDM_SET_TAPE_EAR_20: usize = 1414;
pub const IDM_SET_TAPE_EXPERIENCE: usize = 1415;
pub const IDM_SET_TAPE_INSTANT: usize = 1416;
pub const IDM_SET_MUTE: usize = 1420;
pub const IDM_SET_THROTTLE: usize = 1421;
pub const IDM_SET_ONLINE_TITLES: usize = 1422;
pub const IDM_SET_KEMPSTON_MOUSE: usize = 1423;
pub const IDM_SET_AY_MONO: usize = 1430;
pub const IDM_SET_AY_ACB: usize = 1431;
pub const IDM_SET_AY_ABC: usize = 1432;

/// Debug / inspect
pub const IDM_DBG_TOGGLE: usize = 1501;
pub const IDM_DBG_PAUSE: usize = 1502;
pub const IDM_DBG_CONTINUE: usize = 1503;
pub const IDM_DBG_STEP: usize = 1504;
pub const IDM_DBG_BREAK_PC: usize = 1505;
pub const IDM_DBG_CLEAR_BREAKS: usize = 1506;
pub const IDM_DBG_REFRESH: usize = 1507;

/// Map a Machine→model menu id to [`ModelId`].
#[must_use]
pub fn model_from_menu_id(id: usize) -> Option<ModelId> {
    let idx = id.checked_sub(IDM_MACHINE_MODEL_BASE)?;
    ModelId::ALL.get(idx).copied()
}

/// Menu id for a built-in model.
#[must_use]
pub fn menu_id_for_model(model: ModelId) -> usize {
    ModelId::ALL
        .iter()
        .position(|&m| m == model)
        .map_or(IDM_MACHINE_MODEL_BASE, |i| IDM_MACHINE_MODEL_BASE + i)
}

/// Human label for a built-in model (Win32 menu text).
#[must_use]
pub fn model_menu_label(model: ModelId) -> &'static str {
    match model {
        ModelId::Spectrum16K => "Spectrum 16K",
        ModelId::Spectrum48 => "Spectrum 48K",
        ModelId::Spectrum128 => "Spectrum 128K",
        ModelId::SpectrumPlus2 => "Spectrum +2",
        ModelId::SpectrumPlus2A => "Spectrum +2A",
        ModelId::SpectrumPlus3 => "Spectrum +3",
        ModelId::SpectrumPlus3e => "Spectrum +3e",
        ModelId::Pentagon128 => "Pentagon 128",
        ModelId::ScorpionZs256 => "Scorpion ZS-256",
        ModelId::TimexTC2048 => "Timex TC2048",
        ModelId::TimexTS2068 => "Timex TS2068",
    }
}

/// EAR speed for a tape-speed menu id (`None` for Experience / Instant / unknown).
#[must_use]
pub fn ear_speed_from_menu_id(id: usize) -> Option<u32> {
    match id {
        IDM_SET_TAPE_EAR_1 => Some(1),
        IDM_SET_TAPE_EAR_2 => Some(2),
        IDM_SET_TAPE_EAR_5 => Some(5),
        IDM_SET_TAPE_EAR_10 => Some(10),
        IDM_SET_TAPE_EAR_20 => Some(20),
        _ => None,
    }
}

/// Joystick pref for a Settings→Joystick menu id.
#[must_use]
pub fn joystick_from_menu_id(id: usize) -> Option<PrefJoystick> {
    match id {
        IDM_SET_JOY_KEMPSTON => Some(PrefJoystick::Kempston),
        IDM_SET_JOY_SINCLAIR_L => Some(PrefJoystick::SinclairLeft),
        IDM_SET_JOY_SINCLAIR_R => Some(PrefJoystick::SinclairRight),
        IDM_SET_JOY_CURSOR => Some(PrefJoystick::Cursor),
        _ => None,
    }
}

/// PrefModel mirror of [`model_from_menu_id`].
#[must_use]
pub fn pref_model_from_menu_id(id: usize) -> Option<PrefModel> {
    model_from_menu_id(id).map(PrefModel::from_model_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_menu_ids_round_trip() {
        for m in ModelId::ALL {
            let id = menu_id_for_model(m);
            assert_eq!(model_from_menu_id(id), Some(m));
            assert!(!model_menu_label(m).is_empty());
        }
        assert_eq!(model_from_menu_id(IDM_MACHINE_RESET), None);
    }

    #[test]
    fn ear_speeds_match_prefs_offers() {
        assert_eq!(ear_speed_from_menu_id(IDM_SET_TAPE_EAR_1), Some(1));
        assert_eq!(ear_speed_from_menu_id(IDM_SET_TAPE_EAR_20), Some(20));
        assert_eq!(ear_speed_from_menu_id(IDM_SET_TAPE_EXPERIENCE), None);
        assert_eq!(ear_speed_from_menu_id(IDM_SET_TAPE_INSTANT), None);
    }

    #[test]
    fn joystick_menu_ids_cover_all_prefs() {
        assert_eq!(
            joystick_from_menu_id(IDM_SET_JOY_KEMPSTON),
            Some(PrefJoystick::Kempston)
        );
        assert_eq!(
            joystick_from_menu_id(IDM_SET_JOY_CURSOR),
            Some(PrefJoystick::Cursor)
        );
        assert_eq!(joystick_from_menu_id(IDM_DBG_STEP), None);
    }
}
