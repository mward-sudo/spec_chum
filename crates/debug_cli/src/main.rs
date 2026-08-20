//! Headless debugger: inspect, trace, step, and run-until-break.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use formats::Snapshot48;
use machine::{BreakReason, Machine, Model, TapeLoadOptions, Watch};
use tape::{TapPlayer, TzxPlayer};
use trace::DumpFilter;

#[derive(Parser, Debug)]
#[command(name = "spec-chum-debug", about = "Headless Spec Chum debugger")]
struct Cli {
    /// 48k, 128k, or plus3
    #[arg(long, default_value = "48k")]
    model: String,
    #[arg(long)]
    rom: Option<PathBuf>,
    #[arg(long)]
    tap: Option<PathBuf>,
    #[arg(long)]
    tzx: Option<PathBuf>,
    #[arg(long)]
    snapshot: Option<PathBuf>,
    /// Comma-separated trace categories (tape,cpu,bus,ula,machine,ay,disk,mem,all)
    #[arg(long)]
    trace: Option<String>,
    #[arg(long)]
    json: bool,
    /// Use EAR bitstream loading instead of instant flash-load at LD-BYTES.
    #[arg(long)]
    ear_load: bool,
    /// EAR bitstream speed multiplier (clamped 1..=64; ignored when flash-load is instant).
    #[arg(long, default_value_t = 1)]
    speed: u32,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Run N video frames
    Run {
        #[arg(long, default_value_t = 1)]
        frames: u32,
    },
    /// Run until PC hits this address (hex)
    UntilPc {
        pc: String,
        #[arg(long, default_value_t = 10_000_000)]
        max: u64,
    },
    DumpState,
    DumpTrace {
        #[arg(long)]
        last: Option<usize>,
    },
    Peek {
        addr: String,
        #[arg(long, default_value_t = 64)]
        len: u16,
    },
    Disasm {
        #[arg(long)]
        addr: Option<String>,
        #[arg(long, default_value_t = 16)]
        count: usize,
    },
    BreakPc {
        pc: String,
        #[arg(long, default_value_t = 200)]
        frames: u32,
    },
    TypeLoad {
        #[arg(long)]
        code: bool,
        /// Frames to run after boot before typing LOAD (default 200).
        #[arg(long, default_value_t = 200)]
        warmup: u32,
        /// Max frames after Enter before giving up (default 200 flash / use 200000 for EAR).
        #[arg(long)]
        max: Option<u32>,
    },
    WatchWrite {
        addr: String,
        #[arg(long, default_value_t = 10_000_000)]
        max: u64,
    },
}

fn parse_u16(s: &str) -> Result<u16> {
    let t = s.trim().trim_start_matches('$');
    if let Some(hex) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        return u16::from_str_radix(hex, 16).context("hex u16");
    }
    // 4-digit tokens and anything with A–F are treated as hex (Spectrum-style).
    let looks_hex = t.len() == 4
        || t.bytes()
            .any(|b| b.is_ascii_hexdigit() && !b.is_ascii_digit());
    if looks_hex {
        u16::from_str_radix(t, 16).or_else(|_| t.parse().context("u16"))
    } else {
        t.parse().context("u16")
    }
}

fn default_rom(model: Model) -> PathBuf {
    match model {
        Model::Spectrum48 => PathBuf::from("roms/spec48.rom"),
        Model::Spectrum128 => PathBuf::from("roms/128/spec128uk.rom"),
        Model::SpectrumPlus3 => PathBuf::from("roms/plus3/plus3.rom"),
    }
}

fn parse_model(s: &str) -> Result<Model> {
    Ok(match s.to_ascii_lowercase().as_str() {
        "48" | "48k" => Model::Spectrum48,
        "128" | "128k" => Model::Spectrum128,
        "plus3" | "+3" | "plus2a" => Model::SpectrumPlus3,
        other => bail!("unknown model {other}"),
    })
}

fn load_machine(cli: &Cli) -> Result<Machine> {
    let model = parse_model(&cli.model)?;
    let rom_path = cli.rom.clone().unwrap_or_else(|| default_rom(model));
    let rom = std::fs::read(&rom_path).with_context(|| format!("ROM {}", rom_path.display()))?;
    let mut m = match model {
        Model::Spectrum48 => Machine::new_48k(&rom),
        Model::Spectrum128 => Machine::new_128k(&rom),
        Model::SpectrumPlus3 => Machine::new_plus3(&rom),
    }
    .map_err(|e| anyhow::anyhow!(e))?;
    if let Some(path) = &cli.snapshot {
        let snap = load_snapshot(path)?;
        m.apply_snapshot48(&snap);
    }
    if let Some(path) = &cli.tap {
        let img = tape::TapImage::load(path).map_err(|e| anyhow::anyhow!("{e}"))?;
        m.insert_tape(TapPlayer::new(img));
        m.set_tape_playing(true);
    }
    if let Some(path) = &cli.tzx {
        let data = std::fs::read(path)?;
        let player = TzxPlayer::parse(&data).map_err(|e| anyhow::anyhow!("{e}"))?;
        m.insert_tzx(player);
        m.set_tape_playing(true);
    }
    m.set_tape_load_options(TapeLoadOptions {
        flash_load: !cli.ear_load,
        speed: cli.speed,
    });
    Ok(m)
}

fn attr_mark_code_loaded(m: &Machine) -> bool {
    m.read_mem(0x8000) == 0x21
        && m.read_mem(0x8001) == 0x00
        && m.read_mem(0x8002) == 0x58
        && m.read_mem(0x8003) == 0x36
        && m.read_mem(0x8004) == 0xd7
        && m.read_mem(0x8005) == 0xc9
}

fn print_ok_loaded(m: &Machine) -> bool {
    let prog = u16::from_le_bytes([m.read_mem(0x5C53), m.read_mem(0x5C54)]);
    let eline = u16::from_le_bytes([m.read_mem(0x5C59), m.read_mem(0x5C5A)]);
    for a in prog..eline {
        if m.read_mem(a) == b'O' && m.read_mem(a.wrapping_add(1)) == b'K' {
            return true;
        }
    }
    false
}

fn load_snapshot(path: &Path) -> Result<Snapshot48> {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if ext == "z80" {
        Snapshot48::load_z80(path).map_err(|e| anyhow::anyhow!("{e}"))
    } else {
        Snapshot48::load_sna(path).map_err(|e| anyhow::anyhow!("{e}"))
    }
}

fn print_reason(r: BreakReason, json: bool) {
    if json {
        println!("{{\"break\":\"{r:?}\"}}");
    } else {
        println!("break: {r:?}");
    }
}

fn exit_cli(code: i32) -> ! {
    trace::flush_append();
    std::process::exit(code);
}

fn main() -> Result<()> {
    trace::init_from_env();
    let cli = Cli::parse();
    if let Some(list) = &cli.trace {
        trace::enable(trace::Category::parse_list(list));
    }
    let mut m = load_machine(&cli)?;
    match cli.cmd {
        Cmd::Run { frames } => {
            for _ in 0..frames {
                if m.debugger().paused {
                    break;
                }
                let _ = m.run_frame();
            }
            if cli.json {
                println!("{}", m.inspect().to_json());
            } else {
                print!("{}", m.inspect());
            }
            if m.debugger().last_hit.is_stop() {
                print_reason(m.debugger().last_hit, cli.json);
                exit_cli(2);
            }
        }
        Cmd::UntilPc { pc, max } => {
            let pc = parse_u16(&pc)?;
            m.debugger_mut().add_pc_break(pc);
            let reason = m.run_until_break(max);
            if cli.json {
                println!("{}", m.inspect().to_json());
            } else {
                print!("{}", m.inspect());
            }
            print_reason(reason, cli.json);
            if !matches!(reason, BreakReason::Pc(_)) {
                exit_cli(2);
            }
        }
        Cmd::DumpState => {
            if cli.json {
                println!("{}", m.inspect().to_json());
            } else {
                print!("{}", m.inspect());
            }
        }
        Cmd::DumpTrace { last } => {
            let s = if cli.json {
                trace::dump_json()
            } else if let Some(n) = last {
                trace::dump_filtered(DumpFilter {
                    last_n: Some(n),
                    ..DumpFilter::default()
                })
            } else {
                trace::dump_string()
            };
            print!("{s}");
        }
        Cmd::Peek { addr, len } => {
            let addr = parse_u16(&addr)?;
            print!("{}", m.hexdump(addr, len));
        }
        Cmd::Disasm { addr, count } => {
            let addr = match addr {
                Some(s) => parse_u16(&s)?,
                None => m.cpu().regs.pc,
            };
            print!("{}", m.disasm_window(addr, count));
        }
        Cmd::BreakPc { pc, frames } => {
            let pc = parse_u16(&pc)?;
            m.debugger_mut().add_pc_break(pc);
            for _ in 0..frames {
                if m.debugger().paused {
                    break;
                }
                let _ = m.run_frame();
            }
            if cli.json {
                println!("{}", m.inspect().to_json());
            } else {
                print!("{}", m.inspect());
            }
            print_reason(m.debugger().last_hit, cli.json);
            if !m.debugger().last_hit.is_stop() {
                exit_cli(2);
            }
        }
        Cmd::TypeLoad { code, warmup, max } => {
            if cli.tap.is_none() && cli.tzx.is_none() {
                bail!("type-load requires --tap or --tzx");
            }
            m.set_tape_playing(false);
            for _ in 0..warmup {
                let _ = m.run_frame();
            }
            m.type_load_quotes(code);
            m.set_tape_playing(true);
            let limit = max.unwrap_or(if cli.ear_load { 200_000 } else { 200 });
            let mut loaded = false;
            for _ in 0..limit {
                let _ = m.run_frame();
                loaded = if code {
                    attr_mark_code_loaded(&m)
                } else {
                    print_ok_loaded(&m)
                };
                if loaded {
                    break;
                }
            }
            if cli.json {
                println!("{}", m.inspect().to_json());
                println!(
                    "{{\"load_ok\":{},\"attr_mark\":{}}}",
                    loaded,
                    if code {
                        m.read_mem(0x5800) == 0xd7
                    } else {
                        false
                    }
                );
            } else {
                print!("{}", m.inspect());
                if code {
                    println!("load_ok={loaded} attr_5800={:02X}", m.read_mem(0x5800));
                } else {
                    println!("load_ok={loaded}");
                }
            }
            if !loaded {
                exit_cli(2);
            }
        }
        Cmd::WatchWrite { addr, max } => {
            let addr = parse_u16(&addr)?;
            m.debugger_mut().add_mem_watch(Watch {
                addr,
                read: false,
                write: true,
            });
            let reason = m.run_until_break(max);
            if cli.json {
                println!("{}", m.inspect().to_json());
            } else {
                print!("{}", m.inspect());
            }
            print_reason(reason, cli.json);
            if !matches!(reason, BreakReason::Mem { .. }) {
                exit_cli(2);
            }
        }
    }
    trace::flush_append();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::parse_u16;

    #[test]
    fn parse_u16_hex_prefix_and_dollar() {
        assert_eq!(parse_u16("0x5c00").unwrap(), 0x5c00);
        assert_eq!(parse_u16("0X10").unwrap(), 0x10);
        assert_eq!(parse_u16("$5C00").unwrap(), 0x5c00);
        assert_eq!(parse_u16("5C00").unwrap(), 0x5c00);
        assert_eq!(parse_u16("256").unwrap(), 256);
        assert!(parse_u16("nope").is_err());
    }
}
