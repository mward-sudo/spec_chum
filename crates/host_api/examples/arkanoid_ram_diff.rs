//! Diff Spec Chum's post-load Arkanoid RAM against a Fuse snapshot (#379).
//!
//! Speedlock's `$8230` gate compares 6 bytes at `($94F6)+1` with `"PBRAIN"` at
//! `$9570`; Spec Chum reads zeros there. Diffing the whole `$4000-$FFFF` image
//! against a Fuse-loaded snapshot of the same tape shows whether the EAR path
//! dropped bytes and, if so, exactly which runs.
//!
//! ```bash
//! cargo run -p host_api --release --example arkanoid_ram_diff -- \
//!   tmp/fuse_oracle/arkanoid_fuse_postload.z80 [tzx]
//! ```

use machine::TapeLoadOptions;
use spec_chum_host::{HostSession, ModelId};
use std::env;
use std::path::PathBuf;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn new_session(rom: &[u8]) -> HostSession {
    let mut s = HostSession::new(ModelId::Spectrum48, true);
    s.load_rom_bytes(rom).expect("rom");
    s
}

fn ram_image(s: &HostSession) -> Vec<u8> {
    let m = s.machine().expect("m");
    (0x4000u32..=0xFFFF).map(|a| m.read_mem(a as u16)).collect()
}

fn report_sig(label: &str, ram: &[u8]) {
    let at = |a: u16| ram[usize::from(a) - 0x4000];
    let ptr = u16::from(at(0x94F6)) | (u16::from(at(0x94F7)) << 8);
    let hl = ptr.wrapping_add(1);
    let bytes: Vec<String> = (0..6)
        .map(|i| format!("{:02x}", at(hl.wrapping_add(i))))
        .collect();
    println!(
        "{label}: ($94F6)={ptr:#06x} bytes@{hl:#06x}=[{}] expected=[{}]",
        bytes.join(" "),
        (0..6)
            .map(|i| format!("{:02x}", at(0x9570 + i)))
            .collect::<Vec<_>>()
            .join(" ")
    );
}

fn main() {
    let snap = env::args().nth(1).map_or_else(
        || workspace_root().join("tmp/fuse_oracle/arkanoid_fuse_postload.z80"),
        PathBuf::from,
    );
    let tape = env::args().nth(2).map_or_else(
        || {
            env::var_os("HOME")
                .map(|h| PathBuf::from(h).join("Downloads/Arkanoid.tzx"))
                .expect("home")
        },
        PathBuf::from,
    );

    let rom = std::fs::read(workspace_root().join("roms/spec48.rom")).expect("rom");

    // Fuse reference image.
    let mut fuse = new_session(&rom);
    fuse.load_snapshot(&snap).expect("snap");
    let fuse_ram = ram_image(&fuse);
    println!(
        "fuse snap {} pc={:#06x}",
        snap.display(),
        fuse.machine().expect("m").cpu().regs.pc
    );

    // Spec Chum EAR load of the same tape.
    let mut sc = new_session(&rom);
    sc.open_tape(&tape).expect("open");
    {
        let m = sc.machine_mut().expect("m");
        m.set_tape_load_options(TapeLoadOptions {
            flash_load: false,
            speed: 64,
            experience_load: false,
        });
        m.set_tape_playing(false);
    }
    for _ in 0..200 {
        let _ = sc.machine_mut().expect("m").run_frame();
    }
    {
        let m = sc.machine_mut().expect("m");
        m.type_load_quotes(false);
        m.set_tape_playing(true);
    }
    for _ in 0..40_000u32 {
        let _ = sc.machine_mut().expect("m").run_frame();
        let m = sc.machine().expect("m");
        if m.tape_finished() || !m.tape_playing() {
            break;
        }
    }
    // Settle into the post-tape protection loop so both images are comparable.
    for _ in 0..40 {
        let _ = sc.machine_mut().expect("m").run_frame();
    }
    let sc_ram = ram_image(&sc);
    println!(
        "spec chum pc={:#06x}",
        sc.machine().expect("m").cpu().regs.pc
    );

    report_sig("fuse ", &fuse_ram);
    report_sig("spec ", &sc_ram);

    // Contiguous differing runs, excluding the screen/attr area which both sides
    // keep repainting while the protection loop runs.
    let mut diffs = 0usize;
    let mut runs = 0usize;
    let mut start: Option<usize> = None;
    println!("--- differing runs in $4000-$FFFF ---");
    for i in 0..=sc_ram.len() {
        let differs = i < sc_ram.len() && sc_ram[i] != fuse_ram[i];
        if differs {
            diffs += 1;
        }
        match (differs, start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                runs += 1;
                if runs <= 40 {
                    println!(
                        "  {:#06x}-{:#06x} ({} bytes)",
                        s + 0x4000,
                        i - 1 + 0x4000,
                        i - s
                    );
                }
                start = None;
            }
            _ => {}
        }
    }
    println!("total differing bytes={diffs} runs={runs}");
}
