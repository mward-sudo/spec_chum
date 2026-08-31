//! Headless debugger: inspect, trace, step, and run-until-break.

mod agent_client;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use agent_client::AgentClient;
use agent_server::routes::serve;
use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use control_plane::{ControlPlane, ServerConfig};
use formats::{Snapshot128, Snapshot128Model, Snapshot48};
use machine::{BreakReason, Machine, Model, TapeLoadOptions, Watch};
use spec_chum_host::{HostSession, ModelId};
use tape::{TapPlayer, TzxPlayer};
use trace::DumpFilter;

#[derive(Parser, Debug)]
#[command(name = "spec-chum-debug", about = "Headless Spec Chum debugger")]
struct Cli {
    /// 16k, 48k, 128k, plus2, plus2a, plus3, pentagon, timex/tc2048, ts2068/tc2068
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
    /// EAR speed: N Spectrum frames per run_frame while playing (ignored when flash-load/Instant).
    #[arg(long, default_value_t = 1)]
    speed: u32,
    /// Run the loopback agent HTTP server instead of a one-shot command.
    #[arg(long)]
    serve: bool,
    /// Agent API base URL (or set `SPEC_CHUM_AGENT_URL`) — routes commands over HTTP.
    #[arg(long)]
    agent_url: Option<String>,
    /// Listen port when `--serve` (overrides `SPEC_CHUM_AGENT_PORT`).
    #[arg(long)]
    agent_port: Option<u16>,
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
    machine::resolve_rom_path(model)
        .unwrap_or_else(|| PathBuf::from(machine::rom_candidates(model)[0]))
}

fn parse_model_id(s: &str) -> Result<ModelId> {
    Ok(match s.to_ascii_lowercase().as_str() {
        "16" | "16k" => ModelId::Spectrum16K,
        "48" | "48k" => ModelId::Spectrum48,
        "128" | "128k" => ModelId::Spectrum128,
        "plus2" | "+2" => ModelId::SpectrumPlus2,
        "plus2a" | "+2a" => ModelId::SpectrumPlus2A,
        "plus3" | "+3" => ModelId::SpectrumPlus3,
        "pentagon" | "pentagon128" | "128p" => ModelId::Pentagon128,
        "timex" | "tc2048" | "timex2048" => ModelId::TimexTC2048,
        "ts2068" | "tc2068" | "timex2068" => ModelId::TimexTS2068,
        other => bail!("unknown model {other}"),
    })
}

fn load_host_session(cli: &Cli) -> Result<HostSession> {
    let model = parse_model_id(&cli.model)?;
    let mut session = HostSession::new(model, false);
    if let Some(path) = &cli.rom {
        session.load_rom_path(path)?;
    } else {
        session.select_model(model)?;
    }
    if let Some(path) = &cli.snapshot {
        session.load_snapshot(path)?;
    }
    if let Some(path) = &cli.tap {
        session.open_tape(path)?;
        session.play_tape()?;
    }
    if let Some(path) = &cli.tzx {
        session.open_tape(path)?;
        session.play_tape()?;
    }
    session.set_tape_load_options(TapeLoadOptions {
        flash_load: !cli.ear_load,
        speed: cli.speed,
        ..Default::default()
    })?;
    Ok(session)
}

fn run_serve(cli: &Cli) -> Result<()> {
    let session = load_host_session(cli)?;
    let plane = Arc::new(ControlPlane::with_session(session));
    let mut config = ServerConfig::from_env();
    if let Some(port) = cli.agent_port {
        config.port = port;
    }
    let rt = tokio::runtime::Runtime::new().context("tokio runtime")?;
    rt.block_on(serve(config, plane))
}

fn run_remote(cli: &Cli, client: &AgentClient) -> Result<()> {
    if let Some(list) = &cli.trace {
        client.set_trace_categories(list)?;
    }
    client.set_model(&cli.model)?;
    if cli.ear_load || cli.speed != 1 {
        client.tape_load_options(cli.ear_load, cli.speed)?;
    }
    if let Some(path) = &cli.tap {
        client.tape_open(&path.display().to_string())?;
    }
    if let Some(path) = &cli.tzx {
        client.tape_open(&path.display().to_string())?;
    }
    match &cli.cmd {
        Cmd::Run { frames } => {
            client.run_frames(*frames)?;
            print_remote_inspect(client, cli.json)?;
        }
        Cmd::DumpState => print_remote_inspect(client, cli.json)?,
        Cmd::DumpTrace { last } => {
            let s = client.dump_trace(cli.json, *last)?;
            print!("{s}");
        }
        Cmd::Peek { addr, len } => {
            print!("{}", client.peek(addr, *len)?);
        }
        Cmd::Disasm { addr, count } => {
            print!("{}", client.disasm(addr.as_deref(), *count)?);
        }
        Cmd::TypeLoad { code, warmup, max } => {
            if cli.tap.is_none() && cli.tzx.is_none() {
                bail!("type-load requires --tap or --tzx");
            }
            let body = client.type_load(*code, *warmup, max.unwrap_or(0))?;
            if cli.json {
                println!("{body}");
            } else {
                let load_ok = body
                    .get("load_ok")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                println!("load_ok={load_ok}");
            }
            if !body
                .get("load_ok")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                exit_cli(2);
            }
        }
        other => bail!("remote agent client does not support {other:?} yet"),
    }
    Ok(())
}

fn print_remote_inspect(client: &AgentClient, json: bool) -> Result<()> {
    let text = client.inspect_json()?;
    if json {
        println!("{text}");
    } else {
        // Remote inspect is JSON-only today; still useful for agents.
        println!("{text}");
    }
    Ok(())
}

fn parse_model(s: &str) -> Result<Model> {
    Ok(match s.to_ascii_lowercase().as_str() {
        "16" | "16k" => Model::Spectrum16K,
        "48" | "48k" => Model::Spectrum48,
        "128" | "128k" => Model::Spectrum128,
        "plus2" | "+2" => Model::SpectrumPlus2,
        "plus2a" | "+2a" => Model::SpectrumPlus2A,
        "plus3" | "+3" => Model::SpectrumPlus3,
        "pentagon" | "pentagon128" | "128p" => Model::Pentagon128,
        "timex" | "tc2048" | "timex2048" => Model::TimexTC2048,
        "ts2068" | "tc2068" | "timex2068" => Model::TimexTS2068,
        other => bail!("unknown model {other}"),
    })
}

fn load_machine(cli: &Cli) -> Result<Machine> {
    let model = parse_model(&cli.model)?;
    let rom_path = cli.rom.clone().unwrap_or_else(|| default_rom(model));
    let rom = std::fs::read(&rom_path).with_context(|| format!("ROM {}", rom_path.display()))?;
    let mut m = match model {
        Model::Spectrum16K => Machine::new_16k(&rom),
        Model::Spectrum48 => Machine::new_48k(&rom),
        Model::Spectrum128 => Machine::new_128k(&rom),
        Model::SpectrumPlus2 => Machine::new_plus2(&rom),
        Model::SpectrumPlus2A => Machine::new_plus2a(&rom),
        Model::SpectrumPlus3 => Machine::new_plus3(&rom),
        Model::Pentagon128 => {
            let trdos =
                machine::read_trdos_rom(Model::Pentagon128).map_err(|e| anyhow::anyhow!(e))?;
            Machine::new_pentagon128(&rom, &trdos)
        }
        Model::TimexTC2048 => Machine::new_timex_tc2048(&rom).map_err(|e| e.to_string()),
        Model::TimexTS2068 => {
            let exrom = machine::read_exrom(Model::TimexTS2068).map_err(|e| anyhow::anyhow!(e))?;
            Machine::new_timex_ts2068(&rom, &exrom).map_err(|e| e.to_string())
        }
    }
    .map_err(|e| anyhow::anyhow!(e))?;
    if let Some(path) = &cli.snapshot {
        m = load_and_apply_snapshot(m, path)?;
    }
    if let Some(path) = &cli.tap {
        let img = tape::TapImage::load(path).map_err(|e| anyhow::anyhow!("{e}"))?;
        m.insert_tape(TapPlayer::new(img));
        m.set_tape_playing(true);
    }
    if let Some(path) = &cli.tzx {
        let data = std::fs::read(path)?;
        if tape::TzxPlayer::is_standard_speed_only(&data) {
            let player =
                tape::TzxPlayer::to_tap_player(&data).map_err(|e| anyhow::anyhow!("{e}"))?;
            m.insert_tape(player);
        } else {
            let player = TzxPlayer::parse(&data).map_err(|e| anyhow::anyhow!("{e}"))?;
            m.insert_tzx(player);
        }
        m.set_tape_playing(true);
    }
    m.set_tape_load_options(TapeLoadOptions {
        flash_load: !cli.ear_load,
        speed: cli.speed,
        ..Default::default()
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

fn load_and_apply_snapshot(mut m: Machine, path: &Path) -> Result<Machine> {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if ext == "z80" {
        match Snapshot128::load_z80(path) {
            Ok(snap) => {
                // Z80 v3 encodes +2A/+3 via hw mode (+ modify-hardware bit).
                let want = match snap.model {
                    Snapshot128Model::SpectrumPlus3 => Model::SpectrumPlus3,
                    Snapshot128Model::SpectrumPlus2A => Model::SpectrumPlus2A,
                    Snapshot128Model::Spectrum128 => Model::Spectrum128,
                };
                if m.model() != want {
                    let rom_path = default_rom(want);
                    let rom = std::fs::read(&rom_path)
                        .with_context(|| format!("ROM {}", rom_path.display()))?;
                    m = match want {
                        Model::SpectrumPlus3 => Machine::new_plus3(&rom),
                        Model::SpectrumPlus2A => Machine::new_plus2a(&rom),
                        Model::Spectrum128 => Machine::new_128k(&rom),
                        Model::SpectrumPlus2 => Machine::new_plus2(&rom),
                        Model::Spectrum48
                        | Model::Spectrum16K
                        | Model::TimexTC2048
                        | Model::TimexTS2068
                        | Model::Pentagon128 => {
                            unreachable!("128-family snapshot only")
                        }
                    }
                    .map_err(|e| anyhow::anyhow!(e))?;
                }
                m.apply_snapshot128(&snap);
                Ok(m)
            }
            Err(e128) => match Snapshot48::load_z80(path) {
                Ok(snap) => {
                    if m.model() != Model::Spectrum48 {
                        let rom_path = default_rom(Model::Spectrum48);
                        let rom = std::fs::read(&rom_path)
                            .with_context(|| format!("ROM {}", rom_path.display()))?;
                        m = Machine::new_48k(&rom).map_err(|e| anyhow::anyhow!(e))?;
                    }
                    m.apply_snapshot48(&snap);
                    Ok(m)
                }
                Err(_) => Err(anyhow::anyhow!("{e128}")),
            },
        }
    } else {
        // SNA128 has no 1FFD field — keep the CLI `--model` (cannot auto-detect +3).
        match Snapshot128::load_sna(path) {
            Ok(snap) => {
                m.apply_snapshot128(&snap);
                Ok(m)
            }
            Err(e128) => match Snapshot48::load_sna(path) {
                Ok(snap) => {
                    if m.model() != Model::Spectrum48 {
                        let rom_path = default_rom(Model::Spectrum48);
                        let rom = std::fs::read(&rom_path)
                            .with_context(|| format!("ROM {}", rom_path.display()))?;
                        m = Machine::new_48k(&rom).map_err(|e| anyhow::anyhow!(e))?;
                    }
                    m.apply_snapshot48(&snap);
                    Ok(m)
                }
                Err(_) => Err(anyhow::anyhow!("{e128}")),
            },
        }
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
    if let Err(e) = trace::flush_append() {
        eprintln!("trace append flush failed: {e}");
    }
    std::process::exit(code);
}

fn main() -> Result<()> {
    trace::init_from_env();
    let cli = Cli::parse();
    if let Some(list) = &cli.trace {
        trace::enable(trace::Category::parse_list(list));
    }
    if cli.serve {
        return run_serve(&cli);
    }
    if let Some(url) = &cli.agent_url {
        let token = std::env::var("SPEC_CHUM_AGENT_TOKEN").ok();
        let client = AgentClient::new(url, token)?;
        return run_remote(&cli, &client);
    }
    if std::env::var("SPEC_CHUM_AGENT_URL").is_ok() {
        let client = AgentClient::from_env()?;
        return run_remote(&cli, &client);
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
                println!(
                    "{{\"inspect\":{},\"load_ok\":{},\"attr_mark\":{}}}",
                    m.inspect().to_json(),
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
    trace::flush_append().context("flush SPEC_CHUM_TRACE_APPEND")?;
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
