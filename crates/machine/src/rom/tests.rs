//! Unit tests for the ROM catalog / resolve / install surface.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use bus::{TIMEX_EXROM_SIZE, TRDOS_ROM_SIZE};

use crate::Model;

use super::*;

#[test]
fn all_models_ui_order() {
    assert_eq!(
        ALL_MODELS,
        [
            Model::Spectrum16K,
            Model::Spectrum48,
            Model::Spectrum128,
            Model::SpectrumPlus2,
            Model::SpectrumPlus2A,
            Model::SpectrumPlus3,
            Model::SpectrumPlus3e,
            Model::Pentagon128,
            Model::TimexTC2048,
            Model::TimexTS2068,
        ]
    );
}

#[test]
fn plus3e_uses_fetched_concatenated_rom() {
    assert!(!requires_user_rom(Model::SpectrumPlus3e));
    assert_eq!(
        rom_candidates(Model::SpectrumPlus3e),
        &["roms/plus3e/plus3e.rom"]
    );
    assert_eq!(expected_main_rom_bytes(Model::SpectrumPlus3e), 64 * 1024);
    assert_eq!(model_label(Model::SpectrumPlus3e), "+3e");
    assert!(model_title(Model::SpectrumPlus3e).contains("+3e"));
}

#[test]
fn phase_a_models_use_fetchable_paths() {
    assert!(!requires_user_rom(Model::Spectrum16K));
    assert!(!requires_user_rom(Model::SpectrumPlus2));
    assert_eq!(rom_candidates(Model::Spectrum16K), &["roms/spec48.rom"]);
    assert_eq!(
        rom_candidates(Model::SpectrumPlus2),
        &["roms/plus2/plus2uk.rom"]
    );
}

#[test]
fn pentagon_requires_user_main_and_trdos() {
    assert!(requires_user_rom(Model::Pentagon128));
    assert!(requires_trdos_rom(Model::Pentagon128));
    assert!(!rom_available_in(
        Model::Pentagon128,
        &[PathBuf::from("/nonexistent")]
    ));
}

#[test]
fn rom_availability_matches_filesystem() {
    let roots = search_roots();
    let path = resolve_rom_path_in(Model::Spectrum48, &roots);
    if path.is_some() {
        assert!(rom_available_in(Model::Spectrum48, &roots));
    }
    let plus2 = resolve_rom_path_in(Model::SpectrumPlus2, &roots);
    assert_eq!(
        plus2.is_some(),
        rom_available_in(Model::SpectrumPlus2, &roots)
    );
}

#[test]
fn search_roots_find_bundled_resources_layout() {
    let tmp = std::env::temp_dir().join(format!("spec-chum-rom-roots-{}", std::process::id()));
    let _ = fs::remove_dir_all(&tmp);
    let resources = tmp.join("Contents/Resources");
    let roms = resources.join("roms");
    fs::create_dir_all(&roms).expect("mkdir");
    fs::write(roms.join("spec48.rom"), vec![0u8; 16 * 1024]).expect("rom");
    assert!(resolve_rom_path_in(Model::Spectrum48, std::slice::from_ref(&resources)).is_some());
    let share = tmp.join("share/spec-chum");
    fs::create_dir_all(share.join("roms")).expect("share");
    fs::write(share.join("roms/spec48.rom"), vec![0u8; 16 * 1024]).expect("rom");
    assert!(resolve_rom_path_in(Model::Spectrum48, std::slice::from_ref(&share)).is_some());
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn timex_tc2048_uses_fetchable_rom_path() {
    assert!(!requires_user_rom(Model::TimexTC2048));
    assert_eq!(
        rom_candidates(Model::TimexTC2048),
        &["roms/timex/tc2048.rom"]
    );
}

#[test]
fn timex_ts2068_requires_home_and_exrom_slots() {
    assert!(!requires_user_rom(Model::TimexTS2068));
    assert!(requires_exrom(Model::TimexTS2068));
    assert_eq!(
        rom_candidates(Model::TimexTS2068),
        &["roms/timex/tc2068-0.rom"]
    );
    let slots = rom_slot_descriptors(Model::TimexTS2068);
    assert_eq!(slots.len(), 2);
    assert_eq!(slots[0].id, "main");
    assert_eq!(slots[1].id, "exrom");
    assert_eq!(slots[1].expected_bytes, TIMEX_EXROM_SIZE);
}

#[test]
fn pentagon_has_two_rom_slots() {
    let slots = rom_slot_descriptors(Model::Pentagon128);
    assert_eq!(slots.len(), 2);
    assert_eq!(slots[0].id, "main");
    assert_eq!(slots[1].id, "trdos");
    assert_eq!(slots[1].expected_bytes, TRDOS_ROM_SIZE);
    assert_eq!(slots[1].install_path, TRDOS_ROM_INSTALL_PATH);
    assert!(slots[1]
        .search_paths
        .contains(&"roms/pentagon/trdos-5.04t.rom"));
}

#[test]
fn trdos_native_file_services_rejects_ff_hole() {
    let blank = vec![0u8; TRDOS_ROM_SIZE];
    assert!(
        !trdos_rom_fills_0800_hole(&blank),
        "all-zero 16KiB must not count as a filled hole"
    );
    assert!(!trdos_rom_has_native_file_services(&blank));
    let mut hole = vec![0u8; TRDOS_ROM_SIZE];
    hole[0x08d2] = 0xff;
    hole[0x0d6b] = 0xff;
    assert!(!trdos_rom_has_native_file_services(&hole));
    assert!(!trdos_rom_fills_0800_hole(&hole));
    hole[0x08d2] = 0xe7; // plausible RST #20
    hole[0x0d6b] = 0xc9;
    assert!(trdos_rom_fills_0800_hole(&hole));
    assert!(trdos_rom_has_native_file_services(&hole));
    // Alone Coder 5.04T: VG93 port stub at 08D2h is not a file-load service.
    hole[0x08d2..0x08d7].copy_from_slice(&[0x3e, 0x2c, 0xc3, 0x97, 0x08]);
    assert!(trdos_rom_08d2_is_vg93_port_stub(&hole));
    assert!(trdos_rom_fills_0800_hole(&hole));
    assert!(!trdos_rom_has_native_file_services(&hole));
    assert!(!trdos_rom_has_native_file_services(&[0u8; 8]));
}

#[test]
fn resolve_trdos_prefers_complete_over_hole() {
    let dir = std::env::temp_dir().join(format!("spec_chum_trdos_pref_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let pent = dir.join("roms/pentagon");
    fs::create_dir_all(&pent).expect("tmpdir");
    let mut hole = vec![0u8; TRDOS_ROM_SIZE];
    hole[0x08d2] = 0xff;
    hole[0x0d6b] = 0xff;
    fs::write(pent.join("trdos.rom"), &hole).expect("hole");
    let mut complete = vec![0x00; TRDOS_ROM_SIZE];
    complete[0x08d2] = 0xe7;
    complete[0x0d6b] = 0xc9;
    fs::write(pent.join("trdos-5.04t.rom"), &complete).expect("complete");
    let got = resolve_trdos_rom_preferring_file_services(
        std::slice::from_ref(&dir),
        trdos_rom_candidates(Model::Pentagon128),
    )
    .expect("resolve");
    assert_eq!(got, pent.join("trdos-5.04t.rom"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn install_rom_slot_validates_size() {
    let dir = std::env::temp_dir().join(format!("spec_chum_rom_test_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("tmpdir");
    let bad = dir.join("bad.rom");
    fs::write(&bad, [0u8; 8]).expect("write");
    let err = install_rom_slot(Model::Spectrum48, "main", &bad, std::slice::from_ref(&dir))
        .expect_err("size");
    let msg = err.to_string();
    assert!(msg.contains("16384"), "{msg}");
    assert!(matches!(
        err,
        RomReadError::WrongSize {
            expected: 16384,
            got: 8,
            ..
        }
    ));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn read_trdos_not_applicable_is_typed() {
    let err = read_trdos_rom(Model::Spectrum48).expect_err("48K has no TR-DOS");
    assert!(matches!(err, RomReadError::TrdosNotApplicable { .. }));
    assert!(err.to_string().contains("does not use a TR-DOS ROM"));
}

#[test]
fn install_rom_slot_copies_to_expected_path() {
    let dir = std::env::temp_dir().join(format!("spec_chum_rom_ok_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("tmpdir");
    let src = dir.join("source.rom");
    fs::write(&src, vec![0xAA; 16 * 1024]).expect("write");
    let dest = install_rom_slot(Model::Spectrum48, "main", &src, std::slice::from_ref(&dir))
        .expect("install");
    assert_eq!(dest, dir.join("roms/spec48.rom"));
    assert_eq!(fs::metadata(&dest).expect("meta").len(), 16 * 1024);
    let state = rom_slot_state(
        Model::Spectrum48,
        rom_slot_descriptors(Model::Spectrum48)[0],
        std::slice::from_ref(&dir),
    );
    assert_eq!(state.status, RomSlotStatus::Found);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn override_path_used_when_workspace_empty() {
    let dir = std::env::temp_dir().join(format!("spec_chum_rom_ov_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("tmpdir");
    let src = dir.join("away.rom");
    fs::write(&src, vec![0xCC; 16 * 1024]).expect("write");
    let mut overrides = BTreeMap::new();
    overrides.insert("main".into(), src.clone());
    assert!(rom_available_in_with_overrides(
        Model::Spectrum48,
        std::slice::from_ref(&dir),
        &overrides
    ));
    let _ = fs::remove_dir_all(&dir);
}
