//! Headless debugger: inspect, trace, step, and run-until-break.
//!
//! Local commands route through [`HostSession`] (same backend as `control_plane` / agent HTTP).
//! Set `SPEC_CHUM_AGENT_URL` or `--agent-url` to drive a long-lived agent server instead.

mod agent_client;

use std::path::PathBuf;
use std::sync::Arc;

use agent_client::AgentClient;
use agent_server::routes::serve;
use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use control_plane::{parse_model_slug, ControlPlane, ServerConfig};
use machine::{BreakReason, TapeLoadOptions, Watch};
use spec_chum_host::{HostError, HostSession, ModelId};
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
    /// Insert a `.trd` (attaches Beta / TR-DOS if needed). 48K/128K/Pentagon.
    #[arg(long)]
    trd: Option<PathBuf>,
    /// Load a 16 KiB TR-DOS ROM (attaches Beta on 48K/128K).
    #[arg(long)]
    trdos_rom: Option<PathBuf>,
    /// Comma-separated trace categories (tape,cpu,bus,ula,machine,ay,disk,mem,all)
    #[arg(long)]
    trace: Option<String>,
    #[arg(long)]
    json: bool,
    /// Use EAR bitstream loading instead of instant flash-load at LD-BYTES.
    #[arg(long)]
    ear_load: bool,
    /// EAR speed: N Spectrum frames per `run_frames` while playing (ignored when flash-load/Instant).
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
    cmd: Option<Cmd>,
}

#[derive(Subcommand, Debug, Clone)]
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
        /// Watch an I/O port instead of a memory address.
        #[arg(long)]
        port: bool,
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

fn parse_model_id(s: &str) -> Result<ModelId> {
    parse_model_slug(s).map_err(|e| anyhow::anyhow!(e))
}

fn host_err(e: HostError) -> anyhow::Error {
    anyhow::Error::new(e)
}

fn load_host_session(cli: &Cli) -> Result<HostSession> {
    let model = parse_model_id(&cli.model)?;
    let mut session = HostSession::new(model, false);
    if let Some(path) = &cli.rom {
        session.load_rom_path(path).map_err(host_err)?;
    } else {
        session.select_model(model).map_err(host_err)?;
    }
    if let Some(path) = &cli.snapshot {
        session.load_snapshot(path).map_err(host_err)?;
    }
    if let Some(path) = &cli.trdos_rom {
        session.load_trdos_rom(path).map_err(host_err)?;
    }
    if let Some(path) = &cli.trd {
        session.load_trd(path).map_err(host_err)?;
    }
    if let Some(path) = &cli.tap {
        session.open_tape(path).map_err(host_err)?;
        session.play_tape().map_err(host_err)?;
    }
    if let Some(path) = &cli.tzx {
        session.open_tape(path).map_err(host_err)?;
        session.play_tape().map_err(host_err)?;
    }
    session
        .set_tape_load_options(TapeLoadOptions {
            flash_load: !cli.ear_load,
            speed: cli.speed,
            ..Default::default()
        })
        .map_err(host_err)?;
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
    rt.block_on(serve(config, plane, None))
}

fn run_remote(cli: &Cli, client: &AgentClient, cmd: &Cmd) -> Result<()> {
    if let Some(list) = &cli.trace {
        client.set_trace_categories(list)?;
    }
    if let Some(path) = &cli.rom {
        client.load_rom(&path.display().to_string())?;
    } else {
        client.set_model(&cli.model)?;
    }
    if let Some(path) = &cli.snapshot {
        client.load_snapshot(&path.display().to_string())?;
    }
    if let Some(path) = &cli.trdos_rom {
        client.load_trdos_rom(&path.display().to_string())?;
    }
    if let Some(path) = &cli.trd {
        client.load_trd(&path.display().to_string())?;
    }
    if cli.ear_load || cli.speed != 1 {
        client.tape_load_options(cli.ear_load, cli.speed)?;
    }
    if let Some(path) = &cli.tap {
        client.tape_open(&path.display().to_string())?;
        client.tape_play()?;
    }
    if let Some(path) = &cli.tzx {
        client.tape_open(&path.display().to_string())?;
        client.tape_play()?;
    }
    match cmd {
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
            let load_ok = body
                .get("load_ok")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            if cli.json {
                println!("{body}");
            } else {
                println!("load_ok={load_ok}");
            }
            if !load_ok {
                exit_cli(2);
            }
        }
        Cmd::UntilPc { pc, max } => {
            client.add_breakpoint(pc)?;
            let body = client.run_until(*max)?;
            print_remote_run_result(client, cli.json, &body)?;
            let reason = body
                .get("break_reason")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if !reason.contains("Pc(") {
                exit_cli(2);
            }
        }
        Cmd::BreakPc { pc, frames } => {
            client.add_breakpoint(pc)?;
            let body = client.run_frames(*frames)?;
            print_remote_run_result(client, cli.json, &body)?;
            let reason = body
                .get("break_reason")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if !reason.contains("Pc(") {
                exit_cli(2);
            }
        }
        Cmd::WatchWrite { addr, port, max } => {
            if *port {
                client.add_port_watch_write(addr)?;
            } else {
                client.add_mem_watch_write(addr)?;
            }
            let body = client.run_until(*max)?;
            print_remote_run_result(client, cli.json, &body)?;
            let reason = body
                .get("break_reason")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let expected = if *port { "Port" } else { "Mem" };
            if !reason.contains(expected) {
                exit_cli(2);
            }
        }
    }
    Ok(())
}

fn print_remote_run_result(
    client: &AgentClient,
    json: bool,
    body: &serde_json::Value,
) -> Result<()> {
    if json {
        let inspect = client.inspect_json()?;
        println!("{{\"run\":{body},\"inspect\":{inspect}}}");
    } else {
        let reason = body
            .get("break_reason")
            .and_then(|v| v.as_str())
            .unwrap_or("?");
        println!("break: {reason}");
        print_remote_inspect(client, false)?;
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

fn print_inspect(session: &HostSession, json: bool) -> Result<()> {
    if json {
        println!("{}", session.inspect_json().map_err(host_err)?);
    } else {
        print!("{}", session.inspect_text().map_err(host_err)?);
    }
    Ok(())
}

fn dump_trace_local(json: bool, last: Option<usize>) -> String {
    if json {
        trace::dump_json()
    } else if let Some(n) = last {
        trace::dump_filtered(DumpFilter {
            last_n: Some(n),
            ..DumpFilter::default()
        })
    } else {
        trace::dump_string()
    }
}

fn print_reason(r: BreakReason, json: bool) {
    if json {
        println!("{{\"break\":\"{r:?}\"}}");
    } else {
        println!("break: {r:?}");
    }
}

fn run_local(cli: &Cli, session: &mut HostSession, cmd: &Cmd) -> Result<()> {
    match cmd {
        Cmd::Run { frames } => {
            let reason = session.run_frames(*frames).map_err(host_err)?;
            print_inspect(session, cli.json)?;
            if reason.is_stop() {
                print_reason(reason, cli.json);
                exit_cli(2);
            }
        }
        Cmd::UntilPc { pc, max } => {
            let pc = parse_u16(pc)?;
            session.add_breakpoint(pc).map_err(host_err)?;
            let max = u32::try_from(*max).unwrap_or(u32::MAX);
            let reason = session.run_until_break(max).map_err(host_err)?;
            print_inspect(session, cli.json)?;
            print_reason(reason, cli.json);
            if !matches!(reason, BreakReason::Pc(_)) {
                exit_cli(2);
            }
        }
        Cmd::DumpState => print_inspect(session, cli.json)?,
        Cmd::DumpTrace { last } => {
            let s = dump_trace_local(cli.json, *last);
            print!("{s}");
        }
        Cmd::Peek { addr, len } => {
            let addr = parse_u16(addr)?;
            print!("{}", session.hexdump(addr, *len).map_err(host_err)?);
        }
        Cmd::Disasm { addr, count } => {
            let addr = addr.as_deref().map(parse_u16).transpose()?;
            print!("{}", session.disasm(addr, *count).map_err(host_err)?);
        }
        Cmd::BreakPc { pc, frames } => {
            let pc = parse_u16(pc)?;
            session.add_breakpoint(pc).map_err(host_err)?;
            let reason = session.run_frames(*frames).map_err(host_err)?;
            print_inspect(session, cli.json)?;
            print_reason(reason, cli.json);
            if !matches!(reason, BreakReason::Pc(_)) {
                exit_cli(2);
            }
        }
        Cmd::TypeLoad { code, warmup, max } => {
            if cli.tap.is_none() && cli.tzx.is_none() {
                bail!("type-load requires --tap or --tzx");
            }
            let result = session
                .type_load(*code, *warmup, max.unwrap_or(0))
                .map_err(host_err)?;
            if cli.json {
                let attr_mark = match result.attr_mark {
                    Some(v) => v.to_string(),
                    None => "null".to_string(),
                };
                println!(
                    "{{\"inspect\":{},\"load_ok\":{},\"attr_mark\":{attr_mark}}}",
                    session.inspect_json().map_err(host_err)?,
                    result.load_ok,
                );
            } else {
                print_inspect(session, false)?;
                if *code {
                    let attr = session.peek(0x5800).map_err(host_err)?;
                    println!("load_ok={} attr_5800={attr:02X}", result.load_ok);
                } else {
                    println!("load_ok={}", result.load_ok);
                }
            }
            if !result.load_ok {
                exit_cli(2);
            }
        }
        Cmd::WatchWrite { addr, port, max } => {
            let addr = parse_u16(addr)?;
            if *port {
                session
                    .add_port_watch(Watch {
                        addr,
                        read: false,
                        write: true,
                    })
                    .map_err(host_err)?;
            } else {
                session
                    .add_mem_watch(Watch {
                        addr,
                        read: false,
                        write: true,
                    })
                    .map_err(host_err)?;
            }
            let max = u32::try_from(*max).unwrap_or(u32::MAX);
            let reason = session.run_until_break(max).map_err(host_err)?;
            print_inspect(session, cli.json)?;
            print_reason(reason, cli.json);
            let ok = if *port {
                matches!(reason, BreakReason::Port { .. })
            } else {
                matches!(reason, BreakReason::Mem { .. })
            };
            if !ok {
                exit_cli(2);
            }
        }
    }
    Ok(())
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
    let cmd = cli
        .cmd
        .as_ref()
        .context("subcommand required (or pass --serve to run the HTTP server)")?;
    if let Some(url) = &cli.agent_url {
        let token = std::env::var("SPEC_CHUM_AGENT_TOKEN").ok();
        let client = AgentClient::new(url, token);
        run_remote(&cli, &client, cmd)?;
        trace::flush_append().context("flush SPEC_CHUM_TRACE_APPEND")?;
        return Ok(());
    }
    if std::env::var("SPEC_CHUM_AGENT_URL").is_ok() {
        let client = AgentClient::from_env();
        run_remote(&cli, &client, cmd)?;
        trace::flush_append().context("flush SPEC_CHUM_TRACE_APPEND")?;
        return Ok(());
    }
    let mut session = load_host_session(&cli)?;
    run_local(&cli, &mut session, cmd)?;
    trace::flush_append().context("flush SPEC_CHUM_TRACE_APPEND")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::parse_u16;
    use clap::Parser;
    use std::path::PathBuf;

    use super::{load_host_session, Cli};

    #[test]
    fn serve_without_subcommand() {
        let cli =
            Cli::try_parse_from(["spec-chum-debug", "--serve", "--model", "48k"]).expect("parse");
        assert!(cli.serve);
        assert!(cli.cmd.is_none());
    }

    #[test]
    fn parse_trd_and_trdos_rom_flags() {
        let cli = Cli::try_parse_from([
            "spec-chum-debug",
            "--model",
            "pentagon128",
            "--trd",
            "/tmp/boot.trd",
            "--trdos-rom",
            "/tmp/trdos.rom",
            "dump-state",
        ])
        .expect("parse");
        assert_eq!(cli.model, "pentagon128");
        assert_eq!(cli.trd, Some(PathBuf::from("/tmp/boot.trd")));
        assert_eq!(cli.trdos_rom, Some(PathBuf::from("/tmp/trdos.rom")));
    }

    struct RmTree(PathBuf);
    impl Drop for RmTree {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn load_host_session_trd_attaches_beta() {
        let rom = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../roms/spec48.rom");
        if !rom.is_file() {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        }

        let mut raw = vec![0u8; formats::TRD_SECTOR_SIZE * formats::TRD_SECTORS_PER_TRACK];
        raw[0xe3] = 0; // unknown type → parser infers geometry from length
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let dir = std::env::temp_dir().join(format!(
            "spec_chum_debug_cli_trd_{}_{}",
            std::process::id(),
            nanos
        ));
        std::fs::create_dir_all(&dir).expect("unique temp dir");
        let _cleanup = RmTree(dir.clone());
        let trd_path = dir.join("one_track.trd");
        std::fs::write(&trd_path, &raw).expect("write trd");

        let cli = Cli::try_parse_from([
            "spec-chum-debug",
            "--model",
            "48k",
            "--rom",
            rom.to_str().expect("utf8 rom path"),
            "--trd",
            trd_path.to_str().expect("utf8 trd path"),
            "dump-state",
        ])
        .expect("parse");
        let session = load_host_session(&cli).expect("load session with --trd");
        assert!(session.has_beta(), "Beta should attach when --trd is set");
    }

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
