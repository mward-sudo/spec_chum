use super::*;

fn synthetic_trd_with_marker(b0: u8, b1: u8) -> formats::TrdImage {
    let mut raw = vec![0u8; formats::TRD_SECTOR_SIZE * formats::TRD_SECTORS_PER_TRACK];
    raw[0] = b0;
    raw[1] = b1;
    formats::TrdImage::parse(&raw).unwrap()
}

/// TR-DOS-style `IN A,(#FF)` / `INI` loop at `USR 15616` (`0x3D00`).
fn trdos_read_sector_rom() -> [u8; bus::TRDOS_ROM_SIZE] {
    let mut rom = [0u8; bus::TRDOS_ROM_SIZE];
    let code: &[u8] = &[
        0x3e, 0x3c, // LD A,3Ch
        0xd3, 0xff, // OUT (FFh),A
        0xaf, // XOR A
        0xd3, 0x3f, // OUT (3Fh),A  track 0
        0x3e, 0x00, // LD A,0 — sector 0 (same size as old LD A,1 for jump targets)
        0xd3, 0x5f, // OUT (5Fh),A
        0x3e, 0x80, // LD A,80h
        0xd3, 0x1f, // OUT (1Fh),A
        0x21, 0x00, 0x40, // LD HL,4000h
        0x01, 0x7f, 0x00, // LD BC,007Fh
        0xdb, 0xff, // IN A,(FFh)
        0xe6, 0xc0, // AND C0h
        0x28, 0xfa, // JR Z, wait
        0xfa, 0x22, 0x3d, // JP M, done
        0xed, 0xa2, // INI
        0x18, 0xf3, // JR wait
        0x76, // HALT
    ];
    rom[0x3d00..0x3d00 + code.len()].copy_from_slice(code);
    rom
}

#[test]
fn beta_trdos_rom_loop_reads_trd_sector_into_ram() {
    let mut m = Machine::new_48k(&[0u8; 16384]).unwrap();
    m.load_trdos_rom(&trdos_read_sector_rom()).unwrap();
    m.insert_trd(synthetic_trd_with_marker(0x12, 0x34)).unwrap();
    m.cpu_mut().regs.pc = 0x3d00;
    m.cpu_mut().regs.sp = 0xfffd;
    for _ in 0..100_000 {
        if m.cpu().regs.halted {
            break;
        }
        m.step_once();
    }
    assert!(m.cpu().regs.halted, "synthetic TR-DOS loop should HALT");
    assert_eq!(m.read_mem(0x4000), 0x12);
    assert_eq!(m.read_mem(0x4001), 0x34);
    assert!(m.has_beta());
}

/// Optional: real `roms/trdos.rom` + 48K ROM. Skips cleanly when either is missing.
#[test]
fn trdos_rom_usr_15616_pages_when_fixture_present() {
    let Some(spec) = rom48() else {
        eprintln!("skip: roms/spec48.rom missing");
        return;
    };
    let Some(trdos) = trdos_rom_bytes() else {
        eprintln!("skip: roms/trdos.rom missing (optional #140 TR-DOS boot fixture)");
        return;
    };
    let mut m = Machine::new_48k(&spec).unwrap();
    m.load_trdos_rom(&trdos).unwrap();
    m.insert_trd(synthetic_trd_with_marker(0, 0)).unwrap();
    m.cpu_mut().regs.pc = 0x3d00;
    m.cpu_mut().regs.sp = 0xfffd;
    let mut saw_paged = false;
    for _ in 0..50_000 {
        m.step_once();
        if let Machine::Spec48 { bus, .. } = &m {
            if bus.beta.as_ref().is_some_and(|b| b.paged) {
                saw_paged = true;
                break;
            }
        }
    }
    assert!(
        saw_paged,
        "fetch at 0x3D00 should page TR-DOS ROM (USR 15616)"
    );
}

fn trdos_rom_bytes() -> Option<Vec<u8>> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let path = resolve_trdos_rom_preferring_file_services(
        std::slice::from_ref(&root),
        trdos_rom_candidates(Model::Pentagon128),
    )?;
    let data = std::fs::read(path).ok()?;
    (data.len() == bus::TRDOS_ROM_SIZE).then_some(data)
}

/// Hole-filled 5.04 (or any dump) for the harnessed `19ECh` stand-in path.
/// Prefers `roms/pentagon/trdos.rom` so a complete `trdos-5.04t.rom` does not
/// change the established RUN→boot fixture behaviour. Never returns a dump
/// with native `08D2h`/`0D6Bh` services (those belong on the complete path).
fn trdos_rom_bytes_harness() -> Option<Vec<u8>> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let preferred = [
        "roms/pentagon/trdos.rom",
        "roms/trdos/trdos.rom",
        "roms/trdos.rom",
    ];
    let mut fallback: Option<Vec<u8>> = None;
    for rel in preferred
        .iter()
        .copied()
        .chain(trdos_rom_candidates(Model::Pentagon128).iter().copied())
    {
        let p = root.join(rel);
        let Ok(data) = std::fs::read(&p) else {
            continue;
        };
        if data.len() != bus::TRDOS_ROM_SIZE {
            continue;
        }
        if trdos_rom_fills_0800_hole(&data) {
            continue;
        }
        // Prefer explicit hole-dump paths when present.
        if preferred.contains(&rel) {
            return Some(data);
        }
        if fallback.is_none() {
            fallback = Some(data);
        }
    }
    fallback
}

/// Complete dump only (fills the usual 5.04 `0800h` hole), if present.
fn trdos_rom_bytes_complete() -> Option<Vec<u8>> {
    let data = trdos_rom_bytes()?;
    trdos_rom_fills_0800_hole(&data).then_some(data)
}

/// Synthetic TR-DOS ROM: read track 1 sector 1 (BASIC `boot`) into `8000h`.
fn trdos_read_boot_basic_rom() -> [u8; bus::TRDOS_ROM_SIZE] {
    let mut rom = [0u8; bus::TRDOS_ROM_SIZE];
    let code: &[u8] = &[
        0x3e, 0x3c, // LD A,3Ch
        0xd3, 0xff, // OUT (FFh),A
        0x3e, 0x01, // LD A,1
        0xd3, 0x3f, // OUT (3Fh),A  track 1
        0x3e, 0x00, // LD A,0
        0xd3, 0x5f, // OUT (5Fh),A  sector 0
        0x3e, 0x80, // LD A,80h
        0xd3, 0x1f, // OUT (1Fh),A
        0x21, 0x00, 0x80, // LD HL,8000h
        0x01, 0x7f, 0x00, // LD BC,007Fh
        0xdb, 0xff, // IN A,(FFh)
        0xe6, 0xc0, // AND C0h
        0x28, 0xfa, // JR Z, wait
        0xfa, 0x23, 0x3d, // JP M, HALT (LD A,track is one byte longer than XOR A)
        0xed, 0xa2, // INI
        0x18, 0xf3, // JR wait
        0x76, // HALT
    ];
    rom[0x3d00..0x3d00 + code.len()].copy_from_slice(code);
    rom
}

#[test]
fn beta_reads_synthetic_boot_basic_into_ram() {
    let mut m = Machine::new_48k(&[0u8; 16384]).unwrap();
    m.load_trdos_rom(&trdos_read_boot_basic_rom()).unwrap();
    m.insert_trd(formats::TrdImage::synthetic_trdos_boot_basic())
        .unwrap();
    m.cpu_mut().regs.pc = 0x3d00;
    m.cpu_mut().regs.sp = 0xfffd;
    for _ in 0..4000 {
        if m.cpu().regs.halted {
            break;
        }
        m.step_once();
    }
    assert!(m.cpu().regs.halted, "synthetic TR-DOS loop should HALT");
    assert_eq!(m.read_mem(0x8000), 0x00);
    assert_eq!(m.read_mem(0x8001), 0x0a);
    assert_eq!(m.read_mem(0x8002), 0x17);
    assert_eq!(m.read_mem(0x8003), 0x00);
    assert_eq!(m.read_mem(0x8004), 0xf4); // POKE
    assert_eq!(m.beta_mut().map(|b| b.sector_read_count), Some(1));
}

/// Synthetic TR-DOS ROM: WRITE TRACK one sector then read it back.
fn trdos_write_track_rom() -> [u8; bus::TRDOS_ROM_SIZE] {
    let mut rom = [0u8; bus::TRDOS_ROM_SIZE];
    let code: &[u8] = &[
        0x3e, 0x3c, // LD A,3Ch
        0xd3, 0xff, // OUT (FFh),A
        0xaf, // XOR A
        0xd3, 0x3f, // OUT (3Fh),A  track 0
        0x3e, 0xf0, // LD A,F0h
        0xd3, 0x1f, // OUT (1Fh),A  WRITE TRACK
        0x3e, 0xfe, // ID: FE
        0xd3, 0x7f, // OUT (7Fh),A
        0xaf, // track 0
        0xd3, 0x7f, 0xaf, // side 0
        0xd3, 0x7f, 0x3e, 0x02, // sector 2
        0xd3, 0x7f, 0x3e, 0x01, // 256 bytes
        0xd3, 0x7f, 0x3e, 0xf7, // CRC
        0xd3, 0x7f, 0x3e, 0xfb, // data mark
        0xd3, 0x7f, 0x3e, 0xbe, // fill byte
        0x06, 0x00, // LD B,0  (256 bytes)
        0xd3, 0x7f, // loop: OUT (7Fh),A
        0x10, 0xfc, // DJNZ loop (-4 → 3D29h)
        0x3e, 0xf7, 0xd3, 0x7f, 0x3e, 0xd8, // Force interrupt
        0xd3, 0x1f, 0x3e, 0x02, // read sector ID 2 (VG93 sector register)
        0xd3, 0x5f, 0x3e, 0x80, 0xd3, 0x1f, 0x21, 0x00, 0x60, // HL=6000h
        0x01, 0x7f, 0x00, 0xdb, 0xff, 0xe6, 0xc0, 0x28, 0xfa, 0xfa, 0x50, 0x3d, // JP M, HALT
        0xed, 0xa2, 0x18, 0xf3, 0x76,
    ];
    rom[0x3d00..0x3d00 + code.len()].copy_from_slice(code);
    rom
}

#[test]
fn beta_write_track_via_synthetic_rom() {
    let mut m = Machine::new_48k(&[0u8; 16384]).unwrap();
    m.load_trdos_rom(&trdos_write_track_rom()).unwrap();
    m.insert_trd(synthetic_trd_with_marker(0, 0)).unwrap();
    m.cpu_mut().regs.pc = 0x3d00;
    m.cpu_mut().regs.sp = 0xfffd;
    for _ in 0..50_000 {
        if m.cpu().regs.halted {
            break;
        }
        m.step_once();
    }
    assert!(m.cpu().regs.halted);
    assert_eq!(m.read_mem(0x6000), 0xbe);
    assert_eq!(m.beta_mut().map(|b| b.write_track_count), Some(1));
}

fn init_trdos_usr_call_frame(m: &mut Machine) {
    m.cpu_mut().regs.sp = 0xfffe;
    m.cpu_mut().regs.set_hl(0);
}

fn enter_128k_basic_from_menu(m: &mut Machine) {
    const PRESS: u32 = 15;
    const GAP: u32 = 5;
    for _ in 0..250 {
        let _ = m.run_frame();
    }
    m.hold_keys(&[(0, 0), (4, 4)], PRESS);
    m.hold_keys(&[], GAP);
    m.hold_keys(&[(6, 0)], PRESS);
    m.hold_keys(&[], 30);
    for _ in 0..400 {
        let _ = m.run_frame();
    }
}

fn ensure_trdos_beta128_prog(m: &mut Machine) {
    let prog = u16::from(m.read_mem(0x5c4f)) | (u16::from(m.read_mem(0x5c50)) << 8);
    let chans = u16::from(m.read_mem(0x5c4d)) | (u16::from(m.read_mem(0x5c4e)) << 8);
    if chans == 0 {
        // Keep CHANS clear of `5D25h` sector buffer and PROG at `5E00h`.
        const CHANS: u16 = 0x5f00;
        m.write_mem(0x5c4d, (CHANS & 0xff) as u8);
        m.write_mem(0x5c4e, (CHANS >> 8) as u8);
    }
    // Beta128 `3D21h` requires `(PROG) >= 5D25h`, but `5D25h` is also the TR-DOS
    // 256-byte sector buffer (`1E4Bh` / `197Eh`). Park PROG above that window.
    if prog < 0x5e00 {
        const PROG: u16 = 0x5e00;
        m.write_mem(0x5c4f, (PROG & 0xff) as u8);
        m.write_mem(0x5c50, (PROG >> 8) as u8);
        m.write_mem(0x5c51, ((PROG + 1) & 0xff) as u8);
        m.write_mem(0x5c52, ((PROG + 1) >> 8) as u8);
        m.write_mem(PROG, 0x80);
    }
    // TR-DOS command parse (`3032h` / `02FCh`) reads Spectrum `(PROG)` at `5C59h`,
    // while Beta128 entry checks `5C4Fh`. Keep both pointers on the same line buffer.
    let prog = u16::from(m.read_mem(0x5c4f)) | (u16::from(m.read_mem(0x5c50)) << 8);
    m.write_mem(0x5c59, (prog & 0xff) as u8);
    m.write_mem(0x5c5a, (prog >> 8) as u8);
    // Find-boot (`195Ch`): `LD A,(5CF9); CP #FF; JP NZ,1E3Dh`. Non-`FF` skips the
    // catalog scan and enters load with `B=0` → `1E74h RET Z` (no Type-II). Init
    // copies `(5CF6)→(5CF9)`; `1812h` sets `#FF` on the named-RUN path we may miss.
    m.write_mem(0x5cf6, 0xff);
    m.write_mem(0x5cf9, 0xff);
    m.write_mem(0x5d17, 0xaa);
    m.write_mem(0x5d0f, 0x00);
    m.write_mem(0x5d16, 0x3c);
}

/// Map PC to a TR-DOS ROM offset. [`BetaDisk::read_rom`] only overlays
/// `0000–3FFF`, so any higher PC is RAM and must not be treated as ROM.
fn trdos_rom_pc(pc: u16) -> Option<u16> {
    (pc < 0x4000).then_some(pc)
}

/// `5CC2h` RST `#20` gate used by `2F72h`.
///
/// Stock DOS init writes a lone `C9`. With that stub, `3D94h` `RST #20` / inline
/// `0010h` falls into `JP 3D82h` and recurses (`CALL 3D94h` again). Skip only the
/// `DE==0010h` service so `3D94h` can stay unpatched; other vectors (e.g. `1F54h`)
/// still run. Bytes live in the DOS printer-buffer hole at `5CC2h` (11 bytes).
fn install_trdos_rst20_5cc2_hook(m: &mut Machine) {
    // POP HL / CP L,#10 / JR NZ,do / OR H / RET Z / PUSH HL / RET
    const HOOK: [u8; 11] = [
        0xe1, 0x7d, 0xfe, 0x10, 0x20, 0x03, 0x7c, 0xb7, 0xc8, 0xe5, 0xc9,
    ];
    for (i, &b) in HOOK.iter().enumerate() {
        m.write_mem(0x5cc2 + i as u16, b);
    }
}

/// Harness entry for ROM-gated `RUN` → `boot` (#266 / #140).
///
/// CAT / VG93-wait / PROG-wipe sites stay **stock** — see
/// [`apply_trdos_run_native_abi`]. `3D94h` uses [`install_trdos_rst20_5cc2_hook`].
/// Remaining gap: this 5.04 image has FF from `0800h`–`0E71h` (`08D2h` and
/// `0D6Bh`). Native `012Ah` re-enters catalog before LINE-NEW; `19ECh`
/// VG93+LINE-NEW handoff lives in [`apply_trdos_run_native_abi`].
fn patch_trdos_run_harness_rom(_m: &mut Machine) {
    // No ROM writes — CAT/wait/`19ECh` reductions live in `apply_trdos_run_native_abi`.
}

/// Stock find-boot ABI so catalog ROM stays unpatched (#140 / #266).
///
/// `195Ch` stores caller `DE` as catalog CHS (`1964h`) then `LD C,0` (`1968h`).
/// The sibling entry `1946h` skips that and loads `C` from `(5CDB)`. Seed name
/// `HL=5EE0h`, CHS `DE=0`, one catalog sector `B=1` at `195Ch`, and `C=16` at
/// `196Ah` (after `LD C,0`) so the 16-byte dirent compare / `DJNZ` RET need no
/// ROM writes.
fn apply_trdos_find_boot_native_abi(m: &mut Machine) {
    const NAME: u16 = 0x5ee0;
    match m.cpu().regs.pc {
        0x195c => {
            m.cpu_mut().regs.set_hl(NAME);
            m.cpu_mut().regs.set_de(0);
            m.cpu_mut().regs.b = 1;
        }
        0x196a => {
            m.cpu_mut().regs.set_hl(NAME);
            m.cpu_mut().regs.set_de(0);
            m.cpu_mut().regs.set_bc(0x0110); // B=1, C=16
        }
        _ => {}
    }
}

/// True when the *currently loaded* TR-DOS image has a classic file-load at `08D2h`.
///
/// Must not consult the preferred-on-disk resolver: harness tests load the hole
/// dump even when `trdos-5.04t.rom` exists beside it. Alone Coder 5.04T fills
/// `08D2h` with a VG93 port stub — that is **not** a file service.
fn trdos_rom_has_native_file_services_paged(m: &mut Machine) -> bool {
    let was = m.beta_mut().is_some_and(|b| b.paged);
    if let Some(beta) = m.beta_mut() {
        beta.page_trdos(true);
    }
    let mut img = [0u8; bus::TRDOS_ROM_SIZE];
    for (i, b) in img.iter_mut().enumerate() {
        *b = m.read_mem(i as u16);
    }
    let ok = trdos_rom_has_native_file_services(&img);
    if let Some(beta) = m.beta_mut() {
        beta.page_trdos(was);
    }
    ok
}

/// Register / PC ABI so RUN harness ROM stays stock (#140).
///
/// Replaces former ROM RET/NOP/JR patches at `3D9Dh` / `02D4h` / `213Eh` /
/// `2155h` plus the `3DFFh` A=1 delay seed, and the post-match `19ECh`
/// VG93+LINE-NEW stand-in (this image’s `08D2h` is FF padding).
fn apply_trdos_run_native_abi(m: &mut Machine) {
    apply_trdos_find_boot_native_abi(m);
    let pc = m.cpu().regs.pc;
    match pc {
        // Stock `3DFFh`: `LD C,#FF` / `DEC C` until Z / `DEC A` / JR NZ.
        // Callers use `A=5` (`02C3h`) or `A=#FF` × `B=3` (`3EA4h` motor spin).
        // `A=1` runs one inner 255-iter loop instead of a ROM RET patch.
        0x3dff => {
            m.cpu_mut().regs.a = 1;
        }
        // `3D9Ah` Type-I wait: stock `RST #20`→`1F54h` never unwinds with
        // instant completion — skip to `3DA5h` `POP HL` (was `JR 3DA5h` patch).
        0x3d9d => {
            m.cpu_mut().regs.pc = 0x3da5;
        }
        // Warm `02CBh` `CALL 1D83h` CAT blocks on a key (`161Dh`) — skip the CALL.
        0x02d4 => {
            m.cpu_mut().regs.pc = 0x02d7;
        }
        // 5.04T warm path: `0249h` is `CALL 3AE6h` (XOR A/OUT (9)/LD HL,5D17/RET)
        // where hole 5.04 inlines `LD HL,5D17`. Skip the CALL so SP/IFF stay aligned
        // with the harnessed hole path (native `08D2h` still runs at `19ECh`).
        0x0249
            if m.read_mem(0x0249) == 0xcd
                && m.read_mem(0x024a) == 0xe6
                && m.read_mem(0x024b) == 0x3a =>
        {
            m.cpu_mut().regs.set_hl(0x5d17);
            m.cpu_mut().regs.pc = 0x024c;
        }
        // `213Eh` `CALL Z,211Eh` wipes `(PROG)` when Z (`5D0F=0`); keep seeded
        // `RUN\\r` for `3032h` by skipping the call (was three NOPs).
        0x213e => {
            m.cpu_mut().regs.pc = 0x2141;
        }
        // `2155h` stock `JP 1D90h` (CAT) never returns to `02ECh` — RET to caller.
        0x2155 => {
            let sp = m.cpu().regs.sp;
            let ret = u16::from(m.read_mem(sp)) | (u16::from(m.read_mem(sp.wrapping_add(1))) << 8);
            m.cpu_mut().regs.sp = sp.wrapping_add(2);
            m.cpu_mut().regs.pc = ret;
        }
        // 5.04T: `1FEBh` ends `JP 0897h` instead of hole `OUT (#FF),A / RET`.
        // `CALL 1FEBh` from `3E63h` must return so catalog `1E3Dh` can finish.
        0x1ff3
            if m.read_mem(0x1ff3) == 0xc3
                && m.read_mem(0x1ff4) == 0x97
                && m.read_mem(0x1ff5) == 0x08 =>
        {
            let sys = m.cpu().regs.a;
            if let Some(beta) = m.beta_mut() {
                let _ = beta.out_port(0x00ff, sys);
            }
            let sp = m.cpu().regs.sp;
            let ret = u16::from(m.read_mem(sp)) | (u16::from(m.read_mem(sp.wrapping_add(1))) << 8);
            m.cpu_mut().regs.sp = sp.wrapping_add(2);
            m.cpu_mut().regs.pc = ret;
        }
        // Stock `19ECh`: `RST #20` / `DW 08D2h`. On hole dumps, never enter
        // `08D2h` FF padding — FDC-load `boot` and enter Spectrum `LINE-NEW`.
        // Complete dumps (non-FF at `08D2h`) run the native service.
        0x19ec
            if !trdos_rom_has_native_file_services_paged(m) && trdos_fdc_load_boot_into_prog(m) =>
        {
            m.cpu_mut().regs.pc = 0x1b76;
        }
        _ => {}
    }
}

/// Invoke TR-DOS `RUN` with no filename (loads `boot`).
///
/// Warm entry `0239h`→`02E9h`→`3032h` reaches find-boot Type-II catalog reads.
/// Post-match `19ECh` is stock `RST #20` / inline `08D2h`; this ROM's `08D2h` is
/// FF padding, so [`apply_trdos_run_native_abi`] FDC-loads `boot` at the **call
/// site**, unpages TR-DOS, and enters Spectrum `LINE-NEW` (`1B76h`) — never
/// executes the hole.
/// Name block lives at `5EE0h` so it does not overlap the `5D25h` sector buffer.
fn invoke_trdos_run_boot(m: &mut Machine) -> bool {
    m.write_mem(0x5cb6, 0xf4);
    m.write_mem(0x5cb7, 0x0d);
    // RST #20 epilogue (`2F72h`) enters `5CC2h` before the inline service address.
    // Skip recursive `0010h`→`3D82h` so stock `3D94h` (`RST #20` / `0010h`) returns.
    install_trdos_rst20_5cc2_hook(m);
    m.write_mem(0x5d0f, 0);
    // Find-boot sentinel for `1921h` `CALL Z,195Ch`.
    m.write_mem(0x5d10, 0xff);
    // Seed Spectrum `(PROG)` at `5C59h`: ASCII `RUN` + CR so `3032h` tokenizes.
    let prog = u16::from(m.read_mem(0x5c59)) | (u16::from(m.read_mem(0x5c5a)) << 8);
    for (i, &b) in b"RUN\r\x80".iter().enumerate() {
        m.write_mem(prog.wrapping_add(i as u16), b);
    }
    // Re-assert find-boot catalog gate after `USR 15616` / warm path.
    m.write_mem(0x5cf6, 0xff);
    m.write_mem(0x5cf9, 0xff);
    // Sector count for find-boot outer `B` (`5CDC`) + `1946h` `C` from `(5CDB)`
    // (dirent length; `195Ch` still `LD C,0` and is fixed at `196Ah`).
    m.write_mem(0x5cdb, 0x10);
    m.write_mem(0x5cdc, 0x08);
    // Catalog start CHS (`5CD9` / `5CF4`); `195Ch` `LD (5CF4),DE` needs `DE=0`.
    m.write_mem(0x5cd9, 0x00);
    m.write_mem(0x5cda, 0x00);
    m.write_mem(0x5cf4, 0x00);
    m.write_mem(0x5cf5, 0x00);
    // Full 16-byte TR-DOS dirent for synthetic `boot` — **above** `5D25h` buffer.
    const NAME: u16 = 0x5ee0;
    let dirent: [u8; 16] = [
        b'b', b'o', b'o', b't', b' ', b' ', b' ', b' ', b'B', 28, 0, 27, 0, 1, 0, 1,
    ];
    for (i, &b) in dirent.iter().enumerate() {
        m.write_mem(NAME + i as u16, b);
    }
    m.write_mem(0x5cd7, (NAME & 0xff) as u8);
    m.write_mem(0x5cd8, (NAME >> 8) as u8);
    patch_trdos_run_harness_rom(m);
    let sp = m.cpu().regs.sp;
    m.write_mem(0x5c3d, (sp & 0xff) as u8);
    m.write_mem(0x5c3e, (sp >> 8) as u8);
    // `3D34h` `PUSH HL` after `3D21h` (`HL=5CC2h`), then `3D35h` → `0239h`.
    m.cpu_mut().regs.set_hl(0x5cc2);
    m.cpu_mut().regs.set_de(0);
    m.cpu_mut().regs.sp = sp.wrapping_sub(2);
    m.write_mem(sp.wrapping_sub(2), 0xc2);
    m.write_mem(sp.wrapping_sub(1), 0x5c);
    m.write_mem(0x5d17, 0xaa);
    m.cpu_mut().regs.pc = 0x0239;
    wait_for_trdos_boot_marker(m, 10_000)
}

fn manual_read_track1_sector1(m: &mut Machine) -> bool {
    let Some(beta) = m.beta_mut() else {
        return false;
    };
    beta.page_trdos(true);
    beta.out_port(0x00ff, 0x3c);
    beta.out_port(0x003f, 1);
    beta.out_port(0x005f, 1);
    beta.out_port(0x001f, 0x80);
    if beta.sector_read_count == 0 {
        return false;
    }
    let mut ok = true;
    for _ in 0..256 {
        let Some(st) = beta.in_port(0x001f) else {
            ok = false;
            break;
        };
        if st & 0x02 != 0 {
            let _ = beta.in_port(0x007f);
        }
        if st & 0x80 != 0 {
            break;
        }
    }
    ok
}

/// Enter TR-DOS command mode via `USR 15616` (`3D00h` → `3D31h`).
///
/// Paging is asserted (it works today); reaching the `3D31h` command loop is
/// returned rather than asserted because #140 RUN boot is still open.
fn enter_trdos_command_mode(m: &mut Machine) -> bool {
    init_trdos_usr_call_frame(m);
    m.cpu_mut().regs.pc = 0x3d00;
    let mut saw_paged = false;
    let mut at_prompt = false;
    for _ in 0..20_000_000 {
        m.step_once();
        if m.beta_mut().is_some_and(|b| b.paged) {
            saw_paged = true;
        }
        if saw_paged && trdos_rom_pc(m.cpu().regs.pc) == Some(0x3D31) {
            at_prompt = true;
            break;
        }
    }
    assert!(
        saw_paged,
        "TR-DOS should page (PC={:#06x})",
        m.cpu().regs.pc
    );
    at_prompt
}

/// After find-boot matches `boot`, stock `19ECh` would `RST #20` into `08D2h`,
/// which is FF padding on this ROM image. Load the file body through the real
/// VG93 path into `(PROG)`, wire Spectrum sysvars / `NEWPPC` / FLAGS bit 7
/// (running), unpage TR-DOS, page 48K BASIC ROM, and enter `LINE-NEW` (`1B76h`).
///
/// Why not TR-DOS `012Ah` / native `08D2h` service (this image):
/// - FF padding `0800h`–`0E71h` covers `08D2h` and `0D6Bh` (`012Ah` `CALL 1D97h`).
/// - Entering `012Ah` from `19ECh` re-enters catalog (`30B2h`) / Type-I wait
///   (`3D9Ch`) before LINE-NEW; `RST #20`/`16B0h` is mid-`CALL 166Fh`.
/// - Beta keeps the TR-DOS latch across RAM, so stock `5CC2h`→`1B76h` would still
///   fetch TR-DOS at `1B76h`.
/// - 128/Pentagon ROM0 is the editor; `1B76h` LINE-NEW lives in ROM1 (`7FFDh` bit 4).
fn trdos_fdc_load_boot_into_prog(m: &mut Machine) -> bool {
    let prog = u16::from(m.read_mem(0x5c59)) | (u16::from(m.read_mem(0x5c5a)) << 8);
    let start_sec = m.read_mem(0x5d25 + 14);
    let start_trk = m.read_mem(0x5d25 + 15);
    let file_type = m.read_mem(0x5d25 + 8);
    let len = u16::from(m.read_mem(0x5d25 + 9)) | (u16::from(m.read_mem(0x5d25 + 10)) << 8);
    if file_type != b'B' || start_trk == 0 || len == 0 || len > 256 {
        return false;
    }
    let mut buf = [0u8; 256];
    {
        let Some(beta) = m.beta_mut() else {
            return false;
        };
        beta.page_trdos(true);
        beta.out_port(0x00ff, 0x3c);
        beta.out_port(0x003f, start_trk);
        // TR-DOS dirent sector 0 → VG93 ID 1 (see `BetaDisk::sector_index`).
        beta.out_port(0x005f, start_sec.max(1));
        beta.out_port(0x001f, 0x80);
        if beta.sector_read_count == 0 {
            return false;
        }
        for b in &mut buf {
            let mut spins = 0u32;
            loop {
                let st = beta.in_port(0x001f).unwrap_or(0);
                if st & 0x02 != 0 {
                    break;
                }
                if st & 0x80 != 0 && st & 0x02 == 0 {
                    return false;
                }
                spins += 1;
                if spins > 10_000 {
                    return false;
                }
            }
            *b = beta.in_port(0x007f).unwrap_or(0);
        }
    }
    // Program + empty VARS only (`len`); autostart `AAh` trailer stays out of E_LINE.
    for (i, &b) in buf[..len as usize].iter().enumerate() {
        m.write_mem(prog.wrapping_add(i as u16), b);
    }
    let vars_off = u16::from(m.read_mem(0x5d25 + 11)) | (u16::from(m.read_mem(0x5d25 + 12)) << 8);
    let vars = prog.wrapping_add(vars_off);
    let e_line = vars.wrapping_add(1);
    let write_u16 = |m: &mut Machine, addr: u16, val: u16| {
        m.write_mem(addr, (val & 0xff) as u8);
        m.write_mem(addr.wrapping_add(1), (val >> 8) as u8);
    };
    // Standard Spectrum sysvars (TR-DOS harness aliases `5C4Fh`/`5C59h` differently).
    // Harness parked channel info at `5C4Dh` and PROG at `5C4Fh` — restore CHANS.
    let chans = u16::from(m.read_mem(0x5c4d)) | (u16::from(m.read_mem(0x5c4e)) << 8);
    write_u16(m, 0x5c4b, vars); // VARS
                                // Prefer a live channel block left by 128 BASIC entry. The TR-DOS harness
                                // parks a blank `5F00h` window and aliases CHANS at `5C4Dh` while overwriting
                                // standard `5C4Fh` with PROG — recover the post-menu pointer when present.
    let standard_chans = u16::from(m.read_mem(0x5c4f)) | (u16::from(m.read_mem(0x5c50)) << 8);
    let chans_ptr = if (0x5b00..0x5e00).contains(&standard_chans) {
        standard_chans
    } else if (0x5b00..0x5e00).contains(&chans) {
        chans
    } else {
        // Minimal K/S/R/P channels (5 bytes each: OUT, IN, letter) as NEW installs.
        const CHANS: u16 = 0x5f00;
        let block: [u8; 21] = [
            0xf4, 0x09, 0xa8, 0x10, b'K', // PRINT-OUT / KEY-INPUT
            0xf4, 0x09, 0xc4, 0x15, b'S', // PRINT-OUT / KEY-INPUT
            0x81, 0x0f, 0xc4, 0x15, b'R', // ADD-CHAR / KEY-INPUT
            0xf4, 0x09, 0xc4, 0x15, b'P', // PRINT-OUT / KEY-INPUT
            0x80, // end marker
        ];
        for (i, &b) in block.iter().enumerate() {
            m.write_mem(CHANS.wrapping_add(i as u16), b);
        }
        CHANS
    };
    write_u16(m, 0x5c4f, chans_ptr); // CHANS
    write_u16(m, 0x5c51, chans_ptr); // CURCHL → first channel
                                     // Stream offsets from CHANS (NEW defaults).
    for (i, off) in [0x0001u16, 0x0006, 0x000b, 0x0001, 0x0001, 0x0006, 0x0010]
        .into_iter()
        .enumerate()
    {
        write_u16(m, 0x5c10 + (i as u16) * 2, off);
    }
    write_u16(m, 0x5c53, prog); // PROG
    write_u16(m, 0x5c59, e_line); // E_LINE
    write_u16(m, 0x5c61, e_line); // WORKSP
    write_u16(m, 0x5c63, e_line); // STKBOT
    write_u16(m, 0x5c65, e_line); // STKEND
                                  // Empty edit line terminator at E_LINE (required by many ROM walks).
    m.write_mem(e_line, 0x0d);
    m.write_mem(e_line.wrapping_add(1), 0x80);
    // Autostart LINE from TR-DOS trailer (`AAh`, line LE) or first program line.
    let newppc = if buf.get(len as usize) == Some(&0xaa) {
        u16::from(buf[len as usize + 1]) | (u16::from(buf[len as usize + 2]) << 8)
    } else {
        u16::from(buf[1]) | (u16::from(buf[0]) << 8)
    };
    write_u16(m, 0x5c42, newppc); // NEWPPC
    m.write_mem(0x5c44, 0); // NSPPC = first statement
                            // SYNTAX-Z (`1C11h`) is `BIT 7,(IY+1)`: Z set when bit 7 is
                            // clear, i.e. syntax-checking. TR-DOS/128 editor leftover FLAGS
                            // `1Dh` keeps DECIMAL inserting a second `0x0E` (`00 00 00 80 00`
                            // for 32768) in front of the stored `90…` float → Report C.
    m.write_mem(0x5c3b, m.read_mem(0x5c3b) | 0x80);
    // Leave DOS + select 48K BASIC ROM so `1B76h` is LINE-NEW.
    if let Some(beta) = m.beta_mut() {
        beta.page_trdos(false);
    }
    if let Machine::Spec128 { bus, .. } = m {
        let page = bus.page | 0x10; // bit 4 → ROM1 (48K BASIC)
        bus.out_7ffd(page);
        // Keep BANK_M (`5B5C`) in sync — 128 SWAP at `5B00h` XORs bit 4 from
        // this shadow copy. Desync (e.g. `07` while port is `17`) makes ROM1
        // `3B4Dh`→`0112h` run ROM1 message bytes instead of ROM0 Statement Return.
        m.write_mem(0x5b5c, page);
    }
    true
}

fn wait_for_trdos_boot_marker(m: &mut Machine, max_frames: u32) -> bool {
    let native = trdos_rom_has_native_file_services_paged(m);
    let mut saw_08d2 = false;
    // Prefer instruction steps so we cannot miss the one-instruction `19ECh`
    // call-site window inside a full frame (`apply_trdos_run_native_abi`).
    let max_steps = u64::from(max_frames).saturating_mul(70_000).min(3_000_000);
    for _ in 0..max_steps {
        if m.cpu().regs.pc == 0x08d2 {
            saw_08d2 = true;
        }
        apply_trdos_run_native_abi(m);
        m.step_once();
        if m.read_mem(0x8000) == 0xa5 {
            if native {
                assert!(
                    saw_08d2,
                    "complete ROM: RUN boot must enter native 08D2h file-load service"
                );
            } else {
                assert!(
                    !saw_08d2,
                    "hole dump: RUN boot must not execute 08D2h FF padding (handoff at 19ECh)"
                );
            }
            return true;
        }
    }
    false
}

/// Optional: real `roms/trdos.rom` + 128K main ROM. Skips when either is missing.
#[test]
fn trdos_rom_reads_boot_when_128k_chans_ok_and_fixture_present() {
    let Some(main) = rom_pentagon().or_else(rom128) else {
        eprintln!("skip: pentagon/128 main ROM missing");
        return;
    };
    let Some(trdos) = trdos_rom_bytes_harness() else {
        eprintln!("skip: roms/trdos.rom missing (optional #140 TR-DOS boot fixture)");
        return;
    };
    let mut m = Machine::new_pentagon128(&main, &trdos).unwrap();
    m.insert_trd(formats::TrdImage::synthetic_trdos_boot_basic())
        .unwrap();
    enter_128k_basic_from_menu(&mut m);
    ensure_trdos_beta128_prog(&mut m);
    let at_prompt = enter_trdos_command_mode(&mut m);
    assert!(
        manual_read_track1_sector1(&mut m),
        "FDC should read boot sector after TR-DOS entry (PC={:#06x}, at_prompt={at_prompt})",
        m.cpu().regs.pc
    );
}

/// Debug: dump LINE-NEW handoff sysvars + PC trail until RST `#08` / marker.
#[test]
#[ignore]
fn debug_trdos_line_new_handoff() {
    let Some(main) = rom_pentagon().or_else(rom128) else {
        return;
    };
    let Some(trdos) = trdos_rom_bytes() else {
        return;
    };
    let mut m = Machine::new_pentagon128(&main, &trdos).unwrap();
    m.insert_trd(formats::TrdImage::synthetic_trdos_boot_basic())
        .unwrap();
    enter_128k_basic_from_menu(&mut m);
    ensure_trdos_beta128_prog(&mut m);
    assert!(enter_trdos_command_mode(&mut m));

    // Drive RUN until the harness loads + jumps to LINE-NEW, then stop stepping
    // the DOS path and inspect Spectrum BASIC state.
    m.write_mem(0x5cb6, 0xf4);
    m.write_mem(0x5cb7, 0x0d);
    install_trdos_rst20_5cc2_hook(&mut m);
    m.write_mem(0x5d0f, 0);
    m.write_mem(0x5d10, 0xff);
    let prog0 = u16::from(m.read_mem(0x5c59)) | (u16::from(m.read_mem(0x5c5a)) << 8);
    for (i, &b) in b"RUN\r\x80".iter().enumerate() {
        m.write_mem(prog0.wrapping_add(i as u16), b);
    }
    m.write_mem(0x5cf6, 0xff);
    m.write_mem(0x5cf9, 0xff);
    m.write_mem(0x5cdb, 0x10);
    m.write_mem(0x5cdc, 0x08);
    m.write_mem(0x5cd9, 0x00);
    m.write_mem(0x5cda, 0x00);
    m.write_mem(0x5cf4, 0x00);
    m.write_mem(0x5cf5, 0x00);
    const NAME: u16 = 0x5ee0;
    let dirent: [u8; 16] = [
        b'b', b'o', b'o', b't', b' ', b' ', b' ', b' ', b'B', 28, 0, 27, 0, 1, 0, 1,
    ];
    for (i, &b) in dirent.iter().enumerate() {
        m.write_mem(NAME + i as u16, b);
    }
    m.write_mem(0x5cd7, (NAME & 0xff) as u8);
    m.write_mem(0x5cd8, (NAME >> 8) as u8);
    patch_trdos_run_harness_rom(&mut m);
    let sp = m.cpu().regs.sp;
    m.write_mem(0x5c3d, (sp & 0xff) as u8);
    m.write_mem(0x5c3e, (sp >> 8) as u8);
    m.cpu_mut().regs.set_hl(0x5cc2);
    m.cpu_mut().regs.set_de(0);
    m.cpu_mut().regs.sp = sp.wrapping_sub(2);
    m.write_mem(sp.wrapping_sub(2), 0xc2);
    m.write_mem(sp.wrapping_sub(1), 0x5c);
    m.write_mem(0x5d17, 0xaa);
    m.cpu_mut().regs.pc = 0x0239;

    let mut loaded = false;
    for _ in 0..3_000_000u32 {
        assert_ne!(
            m.cpu().regs.pc,
            0x08d2,
            "debug path must not enter 08D2h FF padding"
        );
        let at_19ec = m.cpu().regs.pc == 0x19ec;
        apply_trdos_run_native_abi(&mut m);
        if at_19ec && m.cpu().regs.pc == 0x1b76 {
            loaded = true;
            break;
        }
        m.step_once();
    }
    assert!(loaded, "did not reach 19ECh FDC handoff");

    let rd16 = |m: &Machine, a: u16| -> u16 {
        u16::from(m.read_mem(a)) | (u16::from(m.read_mem(a.wrapping_add(1))) << 8)
    };
    let page = match &m {
        Machine::Spec128 { bus, .. } => bus.page,
        _ => 0,
    };
    let paged = m.beta_mut().is_some_and(|b| b.paged);
    eprintln!(
        "handoff PC={:#06x} SP={:#06x} IY={:#06x} page={page:#04x} trdos={paged}",
        m.cpu().regs.pc,
        m.cpu().regs.sp,
        m.cpu().regs.iy()
    );
    let stub: Vec<u8> = (0..32).map(|i| m.read_mem(0x5b00 + i)).collect();
    eprintln!("  5B00 stub={stub:02x?}");
    eprintln!(
        "  ERR_NR={:#04x} FLAGS={:#04x} FLAGS2={:#04x} ERR_SP={:#06x} RAMTOP={:#06x}",
        m.read_mem(0x5c3a),
        m.read_mem(0x5c3b),
        m.read_mem(0x5c3c),
        rd16(&m, 0x5c3d),
        rd16(&m, 0x5cb2)
    );
    eprintln!(
        "  NEWPPC={:#06x} NSPPC={:#04x} PPC={:#06x} SUBPPC={:#04x}",
        rd16(&m, 0x5c42),
        m.read_mem(0x5c44),
        rd16(&m, 0x5c45),
        m.read_mem(0x5c47)
    );
    eprintln!(
        "  VARS={:#06x} CHANS={:#06x} PROG={:#06x} E_LINE={:#06x} WORKSP={:#06x} STKEND={:#06x}",
        rd16(&m, 0x5c4b),
        rd16(&m, 0x5c4f),
        rd16(&m, 0x5c53),
        rd16(&m, 0x5c59),
        rd16(&m, 0x5c61),
        rd16(&m, 0x5c65)
    );
    let strms: Vec<u16> = (0..7).map(|i| rd16(&m, 0x5c10 + i * 2)).collect();
    eprintln!("  STRMS={strms:04x?}");
    let prog = rd16(&m, 0x5c53);
    let prog_bytes: Vec<u8> = (0..32).map(|i| m.read_mem(prog.wrapping_add(i))).collect();
    eprintln!("  PROG bytes={prog_bytes:02x?}");
    let chans = rd16(&m, 0x5c4f);
    let chans_bytes: Vec<u8> = (0..21).map(|i| m.read_mem(chans.wrapping_add(i))).collect();
    eprintln!("  CHANS bytes={chans_bytes:02x?}");

    let mut last = 0xffffu16;
    let mut trail: Vec<u16> = Vec::new();
    for step in 0..50_000u32 {
        let pc = m.cpu().regs.pc;
        if pc != last {
            trail.push(pc);
            if trail.len() <= 40 || matches!(pc, 0x0008 | 0x3b4d | 0x0112 | 0x1b76 | 0x1b7d) {
                eprintln!(
                    "step={step} PC={pc:#06x} A={:#04x} HL={:#06x} ERR_NR={:#04x} FLAGS={:#04x}",
                    m.cpu().regs.a,
                    m.cpu().regs.hl(),
                    m.read_mem(0x5c3a),
                    m.read_mem(0x5c3b)
                );
            }
            last = pc;
        }
        if pc == 0x0008 {
            let sp = m.cpu().regs.sp;
            let ret = u16::from(m.read_mem(sp)) | (u16::from(m.read_mem(sp.wrapping_add(1))) << 8);
            let err_byte = m.read_mem(ret);
            let ch_add = rd16(&m, 0x5c5d);
            let prog_now = rd16(&m, 0x5c53);
            let around: Vec<u8> = (0..32)
                .map(|i| m.read_mem(prog_now.wrapping_add(i)))
                .collect();
            let ch_around: Vec<u8> = (0i16..8)
                .map(|i| m.read_mem(ch_add.wrapping_add((i - 2) as u16)))
                .collect();
            eprintln!(
                    "RST8 at step={step} err_byte={err_byte:#04x} CH_ADD={ch_add:#06x} STKEND={:#06x} PROG={prog_now:#06x}",
                    rd16(&m, 0x5c65)
                );
            eprintln!("  PROG now={around:02x?}");
            eprintln!("  near CH_ADD(-2..+5)={ch_around:02x?}");
            eprintln!("  trail={:04x?}", &trail[trail.len().saturating_sub(24)..]);
            break;
        }
        m.step_once();
        if m.read_mem(0x8000) == 0xa5 {
            eprintln!("MARKER at step={step}");
            break;
        }
    }
    eprintln!(
        "final PC={:#06x} 8000={:#04x} ERR_NR={:#04x} page trail_len={} last={:04x?}",
        m.cpu().regs.pc,
        m.read_mem(0x8000),
        m.read_mem(0x5c3a),
        trail.len(),
        &trail[trail.len().saturating_sub(20)..]
    );
}

/// Debug: short PC/FDC trace for #266 RUN path (ignore in CI).
#[test]
#[ignore]
fn debug_trdos_run_pc_trace() {
    let Some(main) = rom_pentagon().or_else(rom128) else {
        return;
    };
    let Some(trdos) = trdos_rom_bytes() else {
        return;
    };
    let mut m = Machine::new_pentagon128(&main, &trdos).unwrap();
    m.insert_trd(formats::TrdImage::synthetic_trdos_boot_basic())
        .unwrap();
    enter_128k_basic_from_menu(&mut m);
    ensure_trdos_beta128_prog(&mut m);
    assert!(enter_trdos_command_mode(&mut m));
    // Same setup as invoke_trdos_run_boot without waiting.
    m.write_mem(0x5cb6, 0xf4);
    m.write_mem(0x5cb7, 0x0d);
    install_trdos_rst20_5cc2_hook(&mut m);
    m.write_mem(0x5d0f, 0);
    m.write_mem(0x5d10, 0xff);
    let prog = u16::from(m.read_mem(0x5c59)) | (u16::from(m.read_mem(0x5c5a)) << 8);
    for (i, &b) in b"RUN\r\x80".iter().enumerate() {
        m.write_mem(prog.wrapping_add(i as u16), b);
    }
    m.write_mem(0x5cf6, 0xff);
    m.write_mem(0x5cf9, 0xff);
    m.write_mem(0x5cdb, 0x10);
    m.write_mem(0x5cdc, 0x08);
    m.write_mem(0x5cd9, 0x00);
    m.write_mem(0x5cda, 0x00);
    m.write_mem(0x5cf4, 0x00);
    m.write_mem(0x5cf5, 0x00);
    const NAME: u16 = 0x5ee0;
    let dirent: [u8; 16] = [
        b'b', b'o', b'o', b't', b' ', b' ', b' ', b' ', b'B', 28, 0, 27, 0, 1, 0, 1,
    ];
    for (i, &b) in dirent.iter().enumerate() {
        m.write_mem(NAME + i as u16, b);
    }
    m.write_mem(0x5cd7, (NAME & 0xff) as u8);
    m.write_mem(0x5cd8, (NAME >> 8) as u8);
    patch_trdos_run_harness_rom(&mut m);
    let sp = m.cpu().regs.sp;
    m.write_mem(0x5c3d, (sp & 0xff) as u8);
    m.write_mem(0x5c3e, (sp >> 8) as u8);
    m.cpu_mut().regs.set_hl(0x5cc2);
    m.cpu_mut().regs.set_de(0);
    m.cpu_mut().regs.sp = sp.wrapping_sub(2);
    m.write_mem(sp.wrapping_sub(2), 0xc2);
    m.write_mem(sp.wrapping_sub(1), 0x5c);
    m.write_mem(0x5d17, 0xaa);
    m.cpu_mut().regs.pc = 0x0239;
    let mut last = 0xffffu16;
    let mut hits = 0u32;
    let mut escaped = 0u32;
    let mut last_ring_len = 0usize;
    let mut saw_c0 = false;
    let watch = [
        0x02e9u16, 0x02ec, 0x2135, 0x2155, 0x3032, 0x030a, 0x031a, 0x1d4d, 0x1d50, 0x1836, 0x187a,
        0x18a1, 0x1921, 0x195c, 0x197e, 0x1997, 0x199c, 0x19dd, 0x08d2, 0x03fa, 0x1e3d, 0x1e40,
        0x1e4d, 0x1e62, 0x1e74, 0x1e75, 0x1e83, 0x3e63, 0x012a, 0x3dc8, 0x3dfa, 0x3f0e, 0x3f25,
        0x07d6, 0x0787,
    ];
    for step in 0..400_000u32 {
        let pc = m.cpu().regs.pc;
        if step % 50_000 == 0 {
            eprintln!(
                "tick step={step} PC={pc:#06x} 5CB6={:#04x} 5D16={:#04x}",
                m.read_mem(0x5cb6),
                m.read_mem(0x5d16)
            );
        }
        if let Some(b) = m.beta_mut() {
            let len = b.command_ring().len();
            if len != last_ring_len {
                eprintln!(
                    "cmd step={step} PC={pc:#06x} ring={:02x?} track={} sys={:#04x} drive={}",
                    b.command_ring(),
                    b.track,
                    b.system,
                    b.system & 3,
                );
                if b.command_ring().last() == Some(&0xc0)
                    || b.command_ring().last().is_some_and(|c| c & 0xe0 == 0x80)
                {
                    saw_c0 |= b.command_ring().last() == Some(&0xc0);
                    hits = 0;
                }
                last_ring_len = len;
            }
        }
        if pc != last {
            let breg = m.cpu().regs.b;
            let sp = m.cpu().regs.sp;
            let sectors = m.beta_mut().map_or(0, |x| x.sector_read_count);
            let in_rom = pc < 0x4000;
            let dos_stub = pc == 0x5cc2 || (0x5c00..0x5e00).contains(&pc);
            let interesting = watch.contains(&pc);
            let limit = if saw_c0 { 250 } else { 80 };
            if interesting || (in_rom && hits < limit) {
                eprintln!("step={step} PC={pc:#06x} B={breg:#04x} SP={sp:#06x} sectors={sectors}");
                if matches!(pc, 0x1e3d | 0x1e40 | 0x1e62 | 0x1e74 | 0x195c | 0x1d4d) {
                    let dir: Vec<u8> = (0..16).map(|i| m.read_mem(0x5d25 + i)).collect();
                    eprintln!(
                        "  A={:#04x} HL={:#06x} 5CF9={:#04x} 5C59→{:#06x} 5D25={:02x?}",
                        m.cpu().regs.a,
                        m.cpu().regs.hl(),
                        m.read_mem(0x5cf9),
                        u16::from(m.read_mem(0x5c59)) | (u16::from(m.read_mem(0x5c5a)) << 8),
                        dir
                    );
                }
                if in_rom && !interesting {
                    hits += 1;
                }
            } else if !in_rom && !dos_stub {
                eprintln!("escaped PC={pc:#06x} at step={step} SP={sp:#06x}");
                escaped += 1;
                if escaped >= 3 {
                    break;
                }
            }
            last = pc;
        }
        apply_trdos_run_native_abi(&mut m);
        m.step_once();
        if m.read_mem(0x8000) == 0xa5 {
            eprintln!("MARKER at step={step}");
            break;
        }
    }
    let pc = m.cpu().regs.pc;
    let marker = m.read_mem(0x8000);
    if let Some(b) = m.beta_mut() {
        eprintln!(
            "final PC={pc:#06x} ring={:02x?} track={} sys={:#04x} sectors={} mem8000={marker:#04x}",
            b.command_ring(),
            b.track,
            b.system,
            b.sector_read_count,
        );
    }
}

/// ROM-gated: stock `3D94h` (`RST #20` / inline `0010h`) returns via the `5CC2h`
/// hook without RET-patching the ROM (#266).
#[test]
fn trdos_3d94_rst20_returns_without_rom_ret_patch_when_fixture_present() {
    let Some(main) = rom_pentagon().or_else(rom128) else {
        eprintln!("skip: pentagon/128 main ROM missing");
        return;
    };
    let Some(trdos) = trdos_rom_bytes() else {
        eprintln!("skip: roms/trdos.rom missing (optional #140 TR-DOS boot fixture)");
        return;
    };
    assert_eq!(
        trdos.get(0x3d94).copied(),
        Some(0xe7),
        "fixture ROM must keep stock RST #20 at 3D94h"
    );
    let mut m = Machine::new_pentagon128(&main, &trdos).unwrap();
    if let Some(beta) = m.beta_mut() {
        beta.page_trdos(true);
    }
    install_trdos_rst20_5cc2_hook(&mut m);
    assert!(
        m.beta_mut().is_some_and(|b| b.paged),
        "TR-DOS must be paged before 3D94h"
    );
    assert_eq!(
        m.read_mem(0x3d94),
        0xe7,
        "paged fetch at 3D94h should be RST #20"
    );
    let at20 = m.read_mem(0x0020);
    assert_eq!(
        at20, 0xc3,
        "paged fetch at 0020h should be TR-DOS JP 2F72h, got {at20:#04x}"
    );
    // RAM trampoline: `CALL 3D94h` / `HALT` — return addr sits below RST #20 traffic.
    const STUB: u16 = 0x8000;
    m.write_mem(STUB, 0xcd);
    m.write_mem(STUB + 1, 0x94);
    m.write_mem(STUB + 2, 0x3d);
    m.write_mem(STUB + 3, 0x76); // HALT
    m.cpu_mut().regs.sp = 0x6000;
    m.cpu_mut().regs.pc = STUB;
    let mut returned = false;
    for _ in 0..50_000 {
        m.step_once();
        if m.cpu().regs.pc == STUB + 3 {
            returned = true;
            break;
        }
    }
    let final_pc = m.cpu().regs.pc;
    let final_paged = m.beta_mut().is_some_and(|b| b.paged);
    assert!(
        returned,
        "3D94h RST #20 should return with 5CC2h hook (PC={final_pc:#06x}, paged={final_paged})"
    );
}

/// ROM-gated: find-boot catalog opcodes stay stock (ABI fixup, not ROM RET/NOP).
#[test]
fn trdos_find_boot_rom_unpatched_when_fixture_present() {
    let Some(main) = rom_pentagon().or_else(rom128) else {
        eprintln!("skip: pentagon/128 main ROM missing");
        return;
    };
    let Some(trdos) = trdos_rom_bytes() else {
        eprintln!("skip: roms/trdos.rom missing (optional #140 TR-DOS boot fixture)");
        return;
    };
    assert_eq!(
        trdos.get(0x1968).copied(),
        Some(0x0e),
        "stock LD C,0 at 1968h"
    );
    assert_eq!(
        trdos.get(0x1977).copied(),
        Some(0xed),
        "stock LD DE,(nn) at 1977h"
    );
    assert_eq!(
        trdos.get(0x1988).copied(),
        Some(0x2a),
        "stock LD HL,(nn) at 1988h"
    );
    assert_eq!(
        trdos.get(0x199a).copied(),
        Some(0x10),
        "stock DJNZ at 199Ah"
    );
    let mut m = Machine::new_pentagon128(&main, &trdos).unwrap();
    if let Some(beta) = m.beta_mut() {
        beta.page_trdos(true);
    }
    patch_trdos_run_harness_rom(&mut m);
    assert_eq!(
        m.read_mem(0x1968),
        0x0e,
        "harness must not patch 1968h LD C,0"
    );
    assert_eq!(
        m.read_mem(0x1977),
        0xed,
        "harness must not patch 1977h LD DE,(5CD9)"
    );
    assert_eq!(
        m.read_mem(0x1988),
        0x2a,
        "harness must not patch 1988h LD HL,(5CD7)"
    );
    assert_eq!(
        m.read_mem(0x199a),
        0x10,
        "harness must not RET-patch 199Ah DJNZ"
    );
}

/// ROM-gated: `3DFFh` delay loop stays stock (A=1 ABI, not ROM RET).
#[test]
fn trdos_3dff_delay_rom_unpatched_when_fixture_present() {
    let Some(main) = rom_pentagon().or_else(rom128) else {
        eprintln!("skip: pentagon/128 main ROM missing");
        return;
    };
    let Some(trdos) = trdos_rom_bytes() else {
        eprintln!("skip: roms/trdos.rom missing (optional #140 TR-DOS boot fixture)");
        return;
    };
    assert_eq!(
        trdos.get(0x3dff).copied(),
        Some(0x0e),
        "stock LD C,#FF at 3DFFh"
    );
    let mut m = Machine::new_pentagon128(&main, &trdos).unwrap();
    if let Some(beta) = m.beta_mut() {
        beta.page_trdos(true);
    }
    patch_trdos_run_harness_rom(&mut m);
    assert_eq!(
        m.read_mem(0x3dff),
        0x0e,
        "harness must not RET-patch 3DFFh delay"
    );
}

/// ROM-gated: CAT / VG93-wait / PROG-wipe sites stay stock (PC/RET ABI).
#[test]
fn trdos_cat_wait_rom_unpatched_when_fixture_present() {
    let Some(main) = rom_pentagon().or_else(rom128) else {
        eprintln!("skip: pentagon/128 main ROM missing");
        return;
    };
    let Some(trdos) = trdos_rom_bytes() else {
        eprintln!("skip: roms/trdos.rom missing (optional #140 TR-DOS boot fixture)");
        return;
    };
    assert_eq!(
        trdos.get(0x3d9d).copied(),
        Some(0xe7),
        "stock RST #20 at 3D9Dh"
    );
    assert_eq!(
        trdos.get(0x02d4).copied(),
        Some(0xcd),
        "stock CALL at 02D4h"
    );
    assert_eq!(
        trdos.get(0x213e).copied(),
        Some(0xcc),
        "stock CALL Z at 213Eh"
    );
    assert_eq!(trdos.get(0x2155).copied(), Some(0xc3), "stock JP at 2155h");
    let mut m = Machine::new_pentagon128(&main, &trdos).unwrap();
    if let Some(beta) = m.beta_mut() {
        beta.page_trdos(true);
    }
    patch_trdos_run_harness_rom(&mut m);
    assert_eq!(
        m.read_mem(0x3d9d),
        0xe7,
        "harness must not JR-patch 3D9Dh wait"
    );
    assert_eq!(
        m.read_mem(0x02d4),
        0xcd,
        "harness must not NOP 02D4h CAT CALL"
    );
    assert_eq!(
        m.read_mem(0x213e),
        0xcc,
        "harness must not NOP 213Eh CALL Z"
    );
    assert_eq!(
        m.read_mem(0x2155),
        0xc3,
        "harness must not RET-patch 2155h JP CAT"
    );
}

/// ROM-gated: `19ECh` stays stock `RST #20`/`08D2h`; padding is never patched.
#[test]
fn trdos_19ec_08d2_callsite_rom_unpatched_when_fixture_present() {
    let Some(main) = rom_pentagon().or_else(rom128) else {
        eprintln!("skip: pentagon/128 main ROM missing");
        return;
    };
    let Some(trdos) = trdos_rom_bytes_harness() else {
        eprintln!("skip: roms/trdos.rom missing (optional #140 TR-DOS boot fixture)");
        return;
    };
    assert_eq!(
        trdos.get(0x19ec).copied(),
        Some(0xe7),
        "stock RST #20 at 19ECh"
    );
    assert_eq!(
        trdos.get(0x19ed).copied(),
        Some(0xd2),
        "stock inline service lo at 19EDh"
    );
    assert_eq!(
        trdos.get(0x19ee).copied(),
        Some(0x08),
        "stock inline service hi → 08D2h"
    );
    let hole = !trdos_rom_fills_0800_hole(&trdos);
    if hole {
        assert_eq!(
            trdos.get(0x08d2).copied(),
            Some(0xff),
            "hole dump has FF padding at 08D2h"
        );
    }
    let mut m = Machine::new_pentagon128(&main, &trdos).unwrap();
    if let Some(beta) = m.beta_mut() {
        beta.page_trdos(true);
    }
    patch_trdos_run_harness_rom(&mut m);
    assert_eq!(
        m.read_mem(0x19ec),
        0xe7,
        "harness must not patch 19ECh RST #20"
    );
    assert_eq!(
        [m.read_mem(0x19ed), m.read_mem(0x19ee)],
        [0xd2, 0x08],
        "harness must not retarget 19ECh service word"
    );
    if hole {
        assert_eq!(
            m.read_mem(0x08d2),
            0xff,
            "harness must not write into 08D2h FF padding"
        );
    } else {
        assert_ne!(
            m.read_mem(0x08d2),
            0xff,
            "complete dump: harness must leave 08D2h service code intact"
        );
    }
}

/// ROM-gated: when a filled-hole dump is present, classify `08D2h`.
///
/// Soft-pass on the usual hole-filled 5.04 image (documents the blocker).
/// Alone Coder **5.04T** fills `0800h`+ with VG93 helpers — `08D2h` is a port
/// stub, not classic file-load — so `19ECh` still uses the FDC stand-in.
#[test]
fn trdos_native_file_services_gate_when_fixture_present() {
    let Some(trdos) = trdos_rom_bytes() else {
        eprintln!("skip: roms/trdos.rom missing (optional #140 TR-DOS boot fixture)");
        return;
    };
    if !trdos_rom_fills_0800_hole(&trdos) {
        assert_eq!(
            trdos.get(0x08d2).copied(),
            Some(0xff),
            "hole dump: 08D2h stays FF"
        );
        assert_eq!(
            trdos.get(0x0d6b).copied(),
            Some(0xff),
            "hole dump: 0D6Bh stays FF"
        );
        eprintln!(
            "trdos native file-services gate: hole dump (0800h–0E71h FF) — \
                 place trdos-5.04t.rom under roms/pentagon/ (Refs #140)"
        );
        return;
    }
    if trdos_rom_08d2_is_vg93_port_stub(&trdos) {
        assert!(!trdos_rom_has_native_file_services(&trdos));
        eprintln!(
            "trdos native file-services gate: 5.04T VG93 port stub at 08D2h — \
                 RUN boot uses 19ECh FDC stand-in after catalog match (Refs #140)"
        );
        return;
    }
    assert!(trdos_rom_has_native_file_services(&trdos));
    assert_eq!(
        [trdos.get(0x19ec), trdos.get(0x19ed), trdos.get(0x19ee)],
        [Some(&0xe7), Some(&0xd2), Some(&0x08)],
        "classic complete dump should keep stock 19ECh → 08D2h"
    );
    eprintln!(
        "trdos native file-services gate: OPEN — classic file-load at 08D2h \
             (native RUN path eligible; Refs #140)"
    );
}

/// ROM-gated: native `012Ah` / `1D97h` stay stock; `0D6Bh` hole is not patched.
#[test]
fn trdos_012a_0d6b_service_rom_unpatched_when_fixture_present() {
    let Some(main) = rom_pentagon().or_else(rom128) else {
        eprintln!("skip: pentagon/128 main ROM missing");
        return;
    };
    let Some(trdos) = trdos_rom_bytes_harness() else {
        eprintln!("skip: roms/trdos.rom missing (optional #140 TR-DOS boot fixture)");
        return;
    };
    assert_eq!(
        trdos.get(0x012a).copied(),
        Some(0xcd),
        "stock CALL at 012Ah"
    );
    assert_eq!(
        [trdos.get(0x012b).copied(), trdos.get(0x012c).copied()],
        [Some(0xe5), Some(0x20)],
        "stock CALL 20E5h at 012Ah"
    );
    assert_eq!(
        [
            trdos.get(0x012d).copied(),
            trdos.get(0x012e).copied(),
            trdos.get(0x012f).copied()
        ],
        [Some(0xcd), Some(0x97), Some(0x1d)],
        "stock CALL 1D97h after 20E5h"
    );
    assert_eq!(
        [
            trdos.get(0x1d97).copied(),
            trdos.get(0x1d98).copied(),
            trdos.get(0x1d99).copied()
        ],
        [Some(0xe7), Some(0x6b), Some(0x0d)],
        "stock RST #20 / 0D6Bh at 1D97h"
    );
    assert_eq!(
        trdos.get(0x1d9a).copied(),
        Some(0xc9),
        "stock RET after 1D97h service word"
    );
    let hole = !trdos_rom_fills_0800_hole(&trdos);
    if hole {
        assert_eq!(
            trdos.get(0x0d6b).copied(),
            Some(0xff),
            "hole dump has FF padding at 0D6Bh (0800h–0E71h)"
        );
    } else {
        assert_ne!(
            trdos.get(0x0d6b).copied(),
            Some(0xff),
            "complete dump has code at 0D6Bh"
        );
    }
    assert_eq!(
        trdos.get(0x16b0).copied(),
        Some(0x16),
        "16B0h is the high byte of CALL 166Fh, not a RST #20 body"
    );
    let mut m = Machine::new_pentagon128(&main, &trdos).unwrap();
    if let Some(beta) = m.beta_mut() {
        beta.page_trdos(true);
    }
    patch_trdos_run_harness_rom(&mut m);
    assert_eq!(
        [m.read_mem(0x012a), m.read_mem(0x012d), m.read_mem(0x1d97)],
        [0xcd, 0xcd, 0xe7],
        "harness must not patch 012Ah / 1D97h"
    );
    if hole {
        assert_eq!(
            m.read_mem(0x0d6b),
            0xff,
            "harness must not write into 0D6Bh FF padding"
        );
    } else {
        assert_ne!(
            m.read_mem(0x0d6b),
            0xff,
            "complete dump: harness must leave 0D6Bh service code intact"
        );
    }
}

/// ROM-gated: TR-DOS `RUN` (no filename) loads synthetic `boot` → `POKE 32768,165`.
#[test]
fn trdos_rom_run_boot_basic_when_fixture_present() {
    let Some(main) = rom_pentagon().or_else(rom128) else {
        eprintln!("skip: pentagon/128 main ROM missing");
        return;
    };
    let Some(trdos) = trdos_rom_bytes_harness() else {
        eprintln!("skip: roms/trdos.rom missing (optional #140 TR-DOS boot fixture)");
        return;
    };
    let mut m = Machine::new_pentagon128(&main, &trdos).unwrap();
    m.insert_trd(formats::TrdImage::synthetic_trdos_boot_basic())
        .unwrap();
    enter_128k_basic_from_menu(&mut m);
    ensure_trdos_beta128_prog(&mut m);
    assert!(
        enter_trdos_command_mode(&mut m),
        "TR-DOS command prompt (3D31h) not reached (PC={:#06x})",
        m.cpu().regs.pc
    );
    let ok = invoke_trdos_run_boot(&mut m);
    let pc = m.cpu().regs.pc;
    let (sectors, track, ring) = m.beta_mut().map_or((0, 0, Vec::new()), |b| {
        (b.sector_read_count, b.track, b.command_ring().to_vec())
    });
    assert!(
            ok,
            "TR-DOS RUN boot should POKE 32768,165 (PC={pc:#06x}, sectors={sectors}, track={track}, ring={ring:02x?}, 8000={:#04x})",
            m.read_mem(0x8000)
        );
    assert_eq!(m.read_mem(0x8000), 0xa5, "boot BASIC POKE 32768,165");
}

/// 5.04T: `08D2h` is a VG93 port stub, so `19ECh` must still take the FDC stand-in.
#[test]
fn trdos_19ec_takes_fdc_standin_when_504t_port_stub_present() {
    let Some(main) = rom_pentagon().or_else(rom128) else {
        eprintln!("skip: pentagon/128 main ROM missing");
        return;
    };
    let Some(trdos) = trdos_rom_bytes_complete() else {
        eprintln!(
            "skip: complete TR-DOS dump missing — place trdos-5.04t.rom / \
                 trdos-complete.rom under roms/pentagon/ (Refs #140)"
        );
        return;
    };
    assert!(trdos_rom_fills_0800_hole(&trdos));
    if !trdos_rom_08d2_is_vg93_port_stub(&trdos) {
        eprintln!("skip: complete dump has classic 08D2h file-load (not 5.04T stub)");
        return;
    }
    assert!(!trdos_rom_has_native_file_services(&trdos));
    let mut m = Machine::new_pentagon128(&main, &trdos).unwrap();
    m.insert_trd(formats::TrdImage::synthetic_trdos_boot_basic())
        .unwrap();
    if let Some(beta) = m.beta_mut() {
        beta.page_trdos(true);
    }
    m.write_mem(0x5d25 + 8, b'B');
    m.write_mem(0x5d25 + 9, 28);
    m.write_mem(0x5d25 + 10, 0);
    m.write_mem(0x5d25 + 14, 1);
    m.write_mem(0x5d25 + 15, 1);
    m.write_mem(0x5c59, 0x00);
    m.write_mem(0x5c5a, 0x5e);
    m.cpu_mut().regs.pc = 0x19ec;
    apply_trdos_run_native_abi(&mut m);
    assert_eq!(
        m.cpu().regs.pc,
        0x1b76,
        "5.04T port-stub 08D2h: 19ECh must FDC-load and enter LINE-NEW"
    );
}

/// 5.04T RUN→boot: Type-II BUSY so catalog `1E3Dh` returns to `1981h`; `08D2h` is a
/// VG93 port stub so `19ECh` uses the FDC/`LINE-NEW` stand-in (hard `0x8000==0xA5`).
#[test]
fn trdos_rom_run_boot_504t_catalog_match_when_complete_present() {
    let Some(main) = rom_pentagon().or_else(rom128) else {
        return;
    };
    let Some(trdos) = trdos_rom_bytes_complete() else {
        eprintln!(
            "skip: complete TR-DOS dump missing — place trdos-5.04t.rom / \
                 trdos-complete.rom under roms/pentagon/ (Refs #140)"
        );
        return;
    };
    if !trdos_rom_08d2_is_vg93_port_stub(&trdos) {
        eprintln!("skip: complete dump has classic 08D2h file-load (not 5.04T stub)");
        return;
    }
    let mut m = Machine::new_pentagon128(&main, &trdos).unwrap();
    m.insert_trd(formats::TrdImage::synthetic_trdos_boot_basic())
        .unwrap();
    enter_128k_basic_from_menu(&mut m);
    ensure_trdos_beta128_prog(&mut m);
    if let Some(beta) = m.beta_mut() {
        beta.page_trdos(true);
    }
    init_trdos_usr_call_frame(&mut m);
    assert!(
        invoke_trdos_run_boot(&mut m),
        "5.04T RUN boot should POKE 32768,165 after catalog match + 19ECh stand-in"
    );
    assert_eq!(m.read_mem(0x8000), 0xa5);
}
