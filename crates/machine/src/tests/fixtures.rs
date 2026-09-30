use super::*;

pub(super) fn rom48() -> Option<Vec<u8>> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../roms/spec48.rom");
    std::fs::read(p).ok()
}

pub(super) fn fixture_tap() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/tape/minimal_code.tap")
}

pub(super) fn rom_pentagon() -> Option<Vec<u8>> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    for rel in ["roms/pentagon/pentagon.rom", "roms/pentagon/128p.rom"] {
        if let Ok(data) = std::fs::read(root.join(rel)) {
            if data.len() == 32768 {
                return Some(data);
            }
        }
    }
    None
}

pub(super) fn rom128() -> Option<Vec<u8>> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../roms/128/spec128uk.rom");
    std::fs::read(p).ok()
}

pub(super) fn rom_plus3() -> Option<Vec<u8>> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../roms");
    for rel in ["plus3/plus3.rom", "plus2a/plus2a.rom"] {
        if let Ok(data) = std::fs::read(root.join(rel)) {
            return Some(data);
        }
    }
    None
}

pub(super) fn rom_plus2a_only() -> Option<Vec<u8>> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../roms/plus2a/plus2a.rom");
    std::fs::read(p).ok()
}

pub(super) fn rom_plus3_only() -> Option<Vec<u8>> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../roms/plus3/plus3.rom");
    std::fs::read(p).ok()
}

pub(super) fn rom_plus3e_only() -> Option<Vec<u8>> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../roms/plus3e/plus3e.rom");
    std::fs::read(p).ok()
}

pub(super) fn rom_plus2() -> Option<Vec<u8>> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../roms/plus2/plus2uk.rom");
    std::fs::read(p).ok()
}

pub(super) fn rom_timex_tc2048() -> Option<Vec<u8>> {
    let path = resolve_rom_path(Model::TimexTC2048)?;
    std::fs::read(path).ok()
}

pub(super) fn rom_timex_ts2068() -> Option<(Vec<u8>, Vec<u8>)> {
    let home = resolve_rom_path(Model::TimexTS2068)?;
    let exrom = resolve_exrom_path(Model::TimexTS2068)?;
    Some((std::fs::read(home).ok()?, std::fs::read(exrom).ok()?))
}
