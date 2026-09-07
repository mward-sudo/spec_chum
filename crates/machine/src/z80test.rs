//! Patrik Rak z80test TAP runner (feature `slow-tests`).
//!
//! Requires `roms/spec48.rom` and fixtures under `tests/fixtures/z80test/`
//! (see that directory’s README, or `./scripts/fetch_z80test.sh`).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use tape::{flash_load_block, TapImage, TapeError};
use thiserror::Error;
use z80::Registers;

use crate::{Machine, MachineBuildError};

/// Spectrum ROM `RST 10` entry (character in A).
const RST10: u16 = 0x0010;
/// Spectrum ROM `CHAN-OPEN`.
const CHAN_OPEN: u16 = 0x1601;
/// Sentinel PC after the test’s final `RET` (simulates return from `USR`).
const USR_RETURN: u16 = 0x0000;
/// z80test load / entry address.
const CODE_ENTRY: u16 = 0x8000;

/// Errors from the z80test TAP harness (`run_z80test_tap`).
#[derive(Debug, Error)]
pub enum Z80testError {
    #[error("missing 48K ROM ({}): run ./scripts/fetch_roms.sh ({source})", path.display())]
    MissingRom {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("TAP {}: {source}", path.display())]
    TapLoad {
        path: PathBuf,
        #[source]
        source: TapeError,
    },
    #[error("CODE data length mismatch: header {header_len}, block {block_len}")]
    CodeLengthMismatch { header_len: usize, block_len: usize },
    #[error("no CODE block found in TAP")]
    NoCodeBlock,
    #[error("expected CODE at {expected:#x}, got {got:#x}")]
    WrongCodeAddress { expected: u16, got: u16 },
    #[error(transparent)]
    Machine(#[from] MachineBuildError),
}

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
fn code_block(image: &TapImage) -> Result<(u16, &[u8]), Z80testError> {
    let mut i = 0;
    while i + 1 < image.blocks.len() {
        let header = &image.blocks[i];
        let data = &image.blocks[i + 1];
        if header.len() >= 18 && header[0] == 0x00 && header[1] == 0x03 {
            let addr = u16::from_le_bytes([header[14], header[15]]);
            let len = u16::from_le_bytes([header[12], header[13]]) as usize;
            if data.len() != len + 2 {
                return Err(Z80testError::CodeLengthMismatch {
                    header_len: len,
                    block_len: data.len(),
                });
            }
            return Ok((addr, data));
        }
        i += 1;
    }
    Err(Z80testError::NoCodeBlock)
}

/// Run a z80test TAP under the 48K machine with RST10 print capture.
pub fn run_z80test_tap(
    tap_path: &Path,
    max_instructions: u64,
) -> Result<Z80testOutcome, Z80testError> {
    let rom_path = rom48_path();
    let rom = std::fs::read(&rom_path).map_err(|source| Z80testError::MissingRom {
        path: rom_path,
        source,
    })?;
    let image = TapImage::load(tap_path).map_err(|source| Z80testError::TapLoad {
        path: tap_path.to_path_buf(),
        source,
    })?;
    let (load_addr, block) = code_block(&image)?;
    if load_addr != CODE_ENTRY {
        return Err(Z80testError::WrongCodeAddress {
            expected: CODE_ENTRY,
            got: load_addr,
        });
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

    /// Instruction budget for z80test on a correct CPU (debug builds are slow).
    /// Real hardware finishes in well under this many ops; inflate for CI debug.
    const Z80TEST_MAX_INSNS: u64 = 5_000_000_000;

    fn assert_z80test_passed(name: &str, tap_name: &str) {
        let tap = fixture_dir().join(tap_name);
        assert!(
            tap.exists(),
            "missing {} — copy from z80test v1.2a or run ./scripts/fetch_z80test.sh",
            tap.display()
        );
        if !rom48_path().exists() {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        }

        let outcome = run_z80test_tap(&tap, Z80TEST_MAX_INSNS).expect("harness error");
        match outcome {
            Z80testOutcome::Passed { output, elapsed } => {
                eprintln!("{name} passed in {elapsed:?}\n{output}");
            }
            Z80testOutcome::Failed {
                output,
                elapsed,
                inspect,
            } => {
                panic!("{name} FAILED in {elapsed:?}\n{inspect}\n{output}");
            }
            Z80testOutcome::TimedOut {
                output,
                elapsed,
                inspect,
            } => {
                panic!(
                    "{name} timed out after {elapsed:?} ({} chars captured)\n{inspect}\n{output}",
                    output.len()
                );
            }
        }
    }

    #[test]
    fn z80doc_all_tests_passed() {
        assert_z80test_passed("z80doc", "z80doc.tap");
    }

    /// Full flag/register suite (`z80full.tap`; also via `./scripts/fetch_z80test.sh`).
    /// `cargo test -p machine --features slow-tests --release z80full_all_tests_passed -- --nocapture`
    #[test]
    fn z80full_all_tests_passed() {
        assert_z80test_passed("z80full", "z80full.tap");
    }

    /// All flags, ignore GPRs (`z80flags.tap`).
    #[test]
    fn z80flags_all_tests_passed() {
        assert_z80test_passed("z80flags", "z80flags.tap");
    }

    /// Documented flags only, ignore GPRs (`z80docflags.tap`).
    #[test]
    fn z80docflags_all_tests_passed() {
        assert_z80test_passed("z80docflags", "z80docflags.tap");
    }

    /// SCF/CCF after every instruction — Q-sensitive Zilog behaviour (`z80ccf.tap`).
    #[test]
    fn z80ccf_all_tests_passed() {
        assert_z80test_passed("z80ccf", "z80ccf.tap");
    }

    /// MEMPTR via `BIT n,(HL)` after each instruction (`z80memptr.tap`).
    #[test]
    fn z80memptr_all_tests_passed() {
        assert_z80test_passed("z80memptr", "z80memptr.tap");
    }

    #[test]
    fn z80test_error_display_preserves_harness_strings() {
        assert_eq!(
            Z80testError::NoCodeBlock.to_string(),
            "no CODE block found in TAP"
        );
        assert_eq!(
            Z80testError::WrongCodeAddress {
                expected: CODE_ENTRY,
                got: 0x9000,
            }
            .to_string(),
            "expected CODE at 0x8000, got 0x9000"
        );
        assert_eq!(
            Z80testError::CodeLengthMismatch {
                header_len: 10,
                block_len: 5,
            }
            .to_string(),
            "CODE data length mismatch: header 10, block 5"
        );
    }

    #[test]
    fn code_block_rejects_empty_tap() {
        let image = TapImage {
            blocks: Vec::new(),
            pause_t: Vec::new(),
        };
        assert!(matches!(code_block(&image), Err(Z80testError::NoCodeBlock)));
    }
}
