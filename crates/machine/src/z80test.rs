//! Patrik Rak z80test TAP runner (feature `slow-tests`).
//!
//! Requires `roms/spec48.rom` and `tests/fixtures/z80test/z80doc.tap`
//! (see that directory’s README, or `./scripts/fetch_z80test.sh`).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use tape::{flash_load_block, TapImage};
use z80::Registers;

use crate::Machine;

/// Spectrum ROM `RST 10` entry (character in A).
const RST10: u16 = 0x0010;
/// Spectrum ROM `CHAN-OPEN`.
const CHAN_OPEN: u16 = 0x1601;
/// Sentinel PC after the test’s final `RET` (simulates return from `USR`).
const USR_RETURN: u16 = 0x0000;
/// z80test load / entry address.
const CODE_ENTRY: u16 = 0x8000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Z80testOutcome {
    Passed {
        output: String,
        elapsed: Duration,
    },
    Failed {
        output: String,
        elapsed: Duration,
        inspect: String,
    },
    TimedOut {
        output: String,
        elapsed: Duration,
        inspect: String,
    },
}

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/z80test")
}

fn rom48_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../roms/spec48.rom")
}

/// Locate the CODE block (type 3) in a z80test TAP: returns (load address, block bytes).
fn code_block(image: &TapImage) -> Result<(u16, &[u8]), String> {
    let mut i = 0;
    while i + 1 < image.blocks.len() {
        let header = &image.blocks[i];
        let data = &image.blocks[i + 1];
        if header.len() >= 18 && header[0] == 0x00 && header[1] == 0x03 {
            let addr = u16::from_le_bytes([header[14], header[15]]);
            let len = u16::from_le_bytes([header[12], header[13]]) as usize;
            if data.len() != len + 2 {
                return Err(format!(
                    "CODE data length mismatch: header {len}, block {}",
                    data.len()
                ));
            }
            return Ok((addr, data));
        }
        i += 1;
    }
    Err("no CODE block found in TAP".into())
}

/// Run a z80test TAP under the 48K machine with RST10 print capture.
pub fn run_z80test_tap(
    tap_path: &std::path::Path,
    max_instructions: u64,
) -> Result<Z80testOutcome, String> {
    let rom = std::fs::read(rom48_path()).map_err(|e| {
        format!(
            "missing 48K ROM ({}): run ./scripts/fetch_roms.sh ({e})",
            rom48_path().display()
        )
    })?;
    let image = TapImage::load(tap_path).map_err(|e| format!("TAP {}: {e}", tap_path.display()))?;
    let (load_addr, block) = code_block(&image)?;
    if load_addr != CODE_ENTRY {
        return Err(format!(
            "expected CODE at {CODE_ENTRY:#x}, got {load_addr:#x}"
        ));
    }

    let mut machine = Machine::new_48k(&rom)?;
    flash_load_block(&mut |addr, v| machine.write_mem(addr, v), block, load_addr);

    // Simulate `RANDOMIZE USR 32768`: CALL into the test, RET lands on USR_RETURN.
    {
        let cpu = machine.cpu_mut();
        cpu.reset();
        cpu.regs = Registers::new();
        cpu.regs.sp = 0xfffe;
        cpu.regs.set_iy(0x5c3a);
        cpu.regs.iff1 = false;
        cpu.regs.iff2 = false;
        cpu.regs.im = 1;
        cpu.regs.pc = CODE_ENTRY;
    }
    machine.push_word(USR_RETURN);

    let mut output = String::new();
    let start = Instant::now();
    let mut instructions = 0u64;

    while instructions < max_instructions {
        let pc = machine.cpu().regs.pc;

        if pc == USR_RETURN {
            break;
        }

        // Stub CHAN-OPEN: printinit JP's here; RET to printinit's caller.
        if pc == CHAN_OPEN {
            machine.ret();
            continue;
        }

        // Capture RST 10 characters without running the ROM print routine.
        if pc == RST10 {
            let ch = machine.cpu().regs.a;
            match ch {
                0x0d => output.push('\n'),
                0x20..=0x7e => output.push(ch as char),
                _ => {}
            }
            machine.ret();
            if output.contains("all tests passed.") || output.contains("tests failed.") {
                break;
            }
            continue;
        }

        machine.step_cpu_only();
        instructions += 1;

        if instructions & 0xff_ffff == 0 {
            // Periodic progress for very long runs (debug builds).
            eprintln!(
                "z80test: {}M instructions, {:.1}s, out_len={}",
                instructions / 1_000_000,
                start.elapsed().as_secs_f32(),
                output.len()
            );
        }
    }

    let elapsed = start.elapsed();
    if output.contains("all tests passed.") {
        Ok(Z80testOutcome::Passed { output, elapsed })
    } else {
        let inspect = machine.inspect().to_string();
        if output.contains("tests failed.") {
            Ok(Z80testOutcome::Failed {
                output,
                elapsed,
                inspect,
            })
        } else {
            Ok(Z80testOutcome::TimedOut {
                output,
                elapsed,
                inspect,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn z80doc_tap() -> PathBuf {
        fixture_dir().join("z80doc.tap")
    }

    /// Instruction budget for z80doc on a correct CPU (debug builds are slow).
    /// Real hardware finishes in well under this many ops; inflate for CI debug.
    const Z80DOC_MAX_INSNS: u64 = 5_000_000_000;

    #[test]
    fn z80doc_all_tests_passed() {
        let tap = z80doc_tap();
        if !tap.exists() {
            panic!(
                "missing {} — copy from z80test v1.2a or run ./scripts/fetch_z80test.sh",
                tap.display()
            );
        }
        if !rom48_path().exists() {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        }

        let outcome = run_z80test_tap(&tap, Z80DOC_MAX_INSNS).expect("harness error");
        match outcome {
            Z80testOutcome::Passed { output, elapsed } => {
                eprintln!("z80doc passed in {elapsed:?}\n{output}");
            }
            Z80testOutcome::Failed {
                output,
                elapsed,
                inspect,
            } => {
                panic!("z80doc FAILED in {elapsed:?}\n{inspect}\n{output}");
            }
            Z80testOutcome::TimedOut {
                output,
                elapsed,
                inspect,
            } => {
                panic!(
                    "z80doc timed out after {elapsed:?} ({} chars captured)\n{inspect}\n{output}",
                    output.len()
                );
            }
        }
    }

    /// Full flag/register suite (`z80full.tap`; also via `./scripts/fetch_z80test.sh`).
    /// `cargo test -p machine --features slow-tests --release z80full_all_tests_passed -- --nocapture`
    #[test]
    fn z80full_all_tests_passed() {
        let tap = fixture_dir().join("z80full.tap");
        if !tap.exists() {
            panic!("missing {} — run ./scripts/fetch_z80test.sh", tap.display());
        }
        if !rom48_path().exists() {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        }
        let outcome = run_z80test_tap(&tap, Z80DOC_MAX_INSNS).expect("harness error");
        match outcome {
            Z80testOutcome::Passed { output, elapsed } => {
                eprintln!("z80full passed in {elapsed:?}\n{output}");
            }
            Z80testOutcome::Failed {
                output,
                elapsed,
                inspect,
            } => {
                panic!("z80full FAILED in {elapsed:?}\n{inspect}\n{output}");
            }
            Z80testOutcome::TimedOut {
                output,
                elapsed,
                inspect,
            } => {
                panic!("z80full timed out after {elapsed:?}\n{inspect}\n{output}");
            }
        }
    }
}
