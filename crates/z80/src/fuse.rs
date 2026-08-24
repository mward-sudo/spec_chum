//! Fuse Z80 test harness (`tests.in` / `tests.expected`).

#![allow(clippy::too_many_lines)]
#![allow(clippy::manual_assert)]
#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::unreadable_literal)]
#![allow(clippy::similar_names)]
#![allow(clippy::items_after_statements)]

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;

use crate::bus::FlatMem;
use crate::cpu::{Cpu, FuseEvent, FuseEventKind};

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/fuse")
}

#[derive(Debug)]
struct TestIn {
    name: String,
    cpu: Cpu,
    mem: FlatMem,
    /// Execute instructions until this many T-states have elapsed (Fuse).
    run_tstates: u64,
}

fn parse_u16(s: &str) -> u16 {
    u16::from_str_radix(s, 16).unwrap_or_else(|_| panic!("bad u16 hex: {s}"))
}

fn parse_u8(s: &str) -> u8 {
    u8::from_str_radix(s, 16).unwrap_or_else(|_| panic!("bad u8 hex: {s}"))
}

fn parse_in_file(path: &std::path::Path) -> Vec<TestIn> {
    let f = File::open(path).expect("tests.in");
    let mut lines = BufReader::new(f).lines().map_while(Result::ok);
    let mut out = Vec::new();
    while let Some(name) = lines.next() {
        let name = name.trim().to_string();
        if name.is_empty() {
            continue;
        }
        let regs = lines.next().expect("regs");
        let mut parts = regs.split_whitespace();
        let mut cpu = Cpu::new();
        cpu.regs.set_af(parse_u16(parts.next().unwrap()));
        cpu.regs.set_bc(parse_u16(parts.next().unwrap()));
        cpu.regs.set_de(parse_u16(parts.next().unwrap()));
        cpu.regs.set_hl(parse_u16(parts.next().unwrap()));
        let af_ = parse_u16(parts.next().unwrap());
        cpu.regs.a_ = (af_ >> 8) as u8;
        cpu.regs.f_ = af_ as u8;
        let bc_ = parse_u16(parts.next().unwrap());
        cpu.regs.b_ = (bc_ >> 8) as u8;
        cpu.regs.c_ = bc_ as u8;
        let de_ = parse_u16(parts.next().unwrap());
        cpu.regs.d_ = (de_ >> 8) as u8;
        cpu.regs.e_ = de_ as u8;
        let hl_ = parse_u16(parts.next().unwrap());
        cpu.regs.h_ = (hl_ >> 8) as u8;
        cpu.regs.l_ = hl_ as u8;
        cpu.regs.set_ix(parse_u16(parts.next().unwrap()));
        cpu.regs.set_iy(parse_u16(parts.next().unwrap()));
        cpu.regs.sp = parse_u16(parts.next().unwrap());
        cpu.regs.pc = parse_u16(parts.next().unwrap());
        cpu.regs.memptr = parse_u16(parts.next().unwrap());

        let misc = lines.next().expect("misc");
        let mut mp = misc.split_whitespace();
        cpu.regs.i = parse_u8(mp.next().unwrap());
        cpu.regs.r = parse_u8(mp.next().unwrap());
        cpu.regs.iff1 = mp.next().unwrap() != "0";
        cpu.regs.iff2 = mp.next().unwrap() != "0";
        cpu.regs.im = mp.next().unwrap().parse().unwrap();
        cpu.regs.halted = mp.next().unwrap() != "0";
        let run_tstates: u64 = mp.next().unwrap().parse().unwrap();

        let mut mem = FlatMem::new();
        loop {
            let line = lines.next().expect("mem");
            let mut toks = line.split_whitespace();
            let first = toks.next().unwrap();
            if first == "-1" {
                break;
            }
            let mut addr = parse_u16(first);
            for t in toks {
                if t == "-1" {
                    break;
                }
                mem.data[addr as usize] = parse_u8(t);
                addr = addr.wrapping_add(1);
            }
        }
        // trailing blank separator often present
        let _ = lines.next();
        out.push(TestIn {
            name,
            cpu,
            mem,
            run_tstates,
        });
    }
    out
}

#[derive(Clone, Debug)]
struct Expected {
    name: String,
    af: u16,
    bc: u16,
    de: u16,
    hl: u16,
    af_: u16,
    bc_: u16,
    de_: u16,
    hl_: u16,
    ix: u16,
    iy: u16,
    sp: u16,
    pc: u16,
    memptr: u16,
    i: u8,
    r: u8,
    iff1: bool,
    iff2: bool,
    im: u8,
    halted: bool,
    tstates: u64,
    mem: Vec<(u16, u8)>,
    events: Vec<FuseEvent>,
}

fn parse_fuse_event_line(line: &str) -> Option<FuseEvent> {
    let mut parts = line.split_whitespace();
    let t: u64 = parts.next()?.parse().ok()?;
    let kind = match parts.next()? {
        "MC" => FuseEventKind::Mc,
        "MR" => FuseEventKind::Mr,
        "MW" => FuseEventKind::Mw,
        "PC" => FuseEventKind::Pc,
        "PR" => FuseEventKind::Pr,
        "PW" => FuseEventKind::Pw,
        _ => return None,
    };
    let addr = parse_u16(parts.next()?);
    let value = parts.next().map(parse_u8);
    Some(FuseEvent {
        t,
        kind,
        addr,
        value,
    })
}

fn format_fuse_event(e: &FuseEvent) -> String {
    match e.value {
        Some(v) => format!("{:5} {} {:04x} {:02x}", e.t, e.kind.as_str(), e.addr, v),
        None => format!("{:5} {} {:04x}", e.t, e.kind.as_str(), e.addr),
    }
}

fn parse_expected(path: &std::path::Path) -> Vec<Expected> {
    let f = File::open(path).expect("tests.expected");
    let mut lines = BufReader::new(f).lines().map_while(Result::ok).peekable();
    let mut out = Vec::new();
    while let Some(name) = lines.next() {
        let name = name.trim().to_string();
        if name.is_empty() {
            continue;
        }
        let mut events = Vec::new();
        while lines
            .peek()
            .is_some_and(|l| l.starts_with(' ') || l.starts_with('\t'))
        {
            let line = lines.next().unwrap();
            let ev = parse_fuse_event_line(&line)
                .unwrap_or_else(|| panic!("{name}: unparsable event line: {line:?}"));
            events.push(ev);
        }
        let regs = lines.next().expect("exp regs");
        let mut parts = regs.split_whitespace();
        let af = parse_u16(parts.next().unwrap());
        let bc = parse_u16(parts.next().unwrap());
        let de = parse_u16(parts.next().unwrap());
        let hl = parse_u16(parts.next().unwrap());
        let af_ = parse_u16(parts.next().unwrap());
        let bc_ = parse_u16(parts.next().unwrap());
        let de_ = parse_u16(parts.next().unwrap());
        let hl_ = parse_u16(parts.next().unwrap());
        let ix = parse_u16(parts.next().unwrap());
        let iy = parse_u16(parts.next().unwrap());
        let sp = parse_u16(parts.next().unwrap());
        let pc = parse_u16(parts.next().unwrap());
        let memptr = parse_u16(parts.next().unwrap());

        let misc = lines.next().expect("exp misc");
        let mut mp = misc.split_whitespace();
        let i = parse_u8(mp.next().unwrap());
        let r = parse_u8(mp.next().unwrap());
        let iff1 = mp.next().unwrap() != "0";
        let iff2 = mp.next().unwrap() != "0";
        let im = mp.next().unwrap().parse().unwrap();
        let halted = mp.next().unwrap() != "0";
        let tstates = mp.next().unwrap().parse().unwrap();

        let mut mem = Vec::new();
        while let Some(line) = lines.peek() {
            let t = line.trim();
            if t.is_empty() {
                lines.next();
                break;
            }
            // next test name: no spaces and not ending pattern — names don't contain spaces
            if !t.contains(' ')
                && t != "-1"
                && !t.chars().all(|c| c.is_ascii_hexdigit() || c == '_')
            {
                // could be next name without blank — Fuse uses blank between tests
                break;
            }
            if t.chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic() && !t.contains(' '))
                && !t.chars().all(|c| c.is_ascii_hexdigit() || c == '_')
            {
                break;
            }
            let line = lines.next().unwrap();
            let mut toks = line.split_whitespace();
            let Some(first) = toks.next() else { break };
            if first == "-1" {
                let _ = lines.next(); // blank
                break;
            }
            // If this looks like a test name (no spaces already handled)
            if toks.clone().next().is_none() && first.chars().any(|c| c.is_ascii_alphabetic()) {
                // Actually we already consumed — shouldn't happen
                break;
            }
            let mut addr = parse_u16(first);
            for t in toks {
                if t == "-1" {
                    break;
                }
                mem.push((addr, parse_u8(t)));
                addr = addr.wrapping_add(1);
            }
        }

        out.push(Expected {
            name,
            af,
            bc,
            de,
            hl,
            af_,
            bc_,
            de_,
            hl_,
            ix,
            iy,
            sp,
            pc,
            memptr,
            i,
            r,
            iff1,
            iff2,
            im,
            halted,
            tstates,
            mem,
            events,
        });
    }
    out
}

/// Flat memory + Fuse I/O for test vectors (IN returns port high byte).
struct FuseBus {
    mem: FlatMem,
}

impl crate::bus::Memory for FuseBus {
    fn read(&mut self, addr: u16, t: u64) -> (u8, u32) {
        self.mem.read(addr, t)
    }
    fn write(&mut self, addr: u16, value: u8, t: u64) -> u32 {
        self.mem.write(addr, value, t)
    }
}

impl crate::bus::Io for FuseBus {
    fn in_port(&mut self, port: u16, _t: u64) -> (u8, u32) {
        ((port >> 8) as u8, 0)
    }
    fn out_port(&mut self, _port: u16, _value: u8, _t: u64) -> u32 {
        0
    }
}

fn run_case(tin: &TestIn, exp: &Expected) -> Result<(), String> {
    let mut cpu = tin.cpu.clone();
    cpu.fuse_log = Some(Vec::new());
    let mut bus = FuseBus {
        mem: tin.mem.clone(),
    };
    let start = cpu.t;
    // Fuse: run until at least `run_tstates` have elapsed (instruction boundaries).
    while cpu.t - start < tin.run_tstates {
        let dt = cpu.step(&mut bus);
        if dt == 0 {
            break;
        }
        // Safety against runaway
        if cpu.t - start > tin.run_tstates.saturating_add(100_000) {
            break;
        }
    }
    let mut errs = Vec::new();
    macro_rules! check {
        ($name:expr, $got:expr, $want:expr) => {
            if $got != $want {
                errs.push(format!("{}: got {:04X} want {:04X}", $name, $got, $want));
            }
        };
    }
    check!("AF", cpu.regs.af(), exp.af);
    check!("BC", cpu.regs.bc(), exp.bc);
    check!("DE", cpu.regs.de(), exp.de);
    check!("HL", cpu.regs.hl(), exp.hl);
    check!(
        "AF_",
        u16::from(cpu.regs.a_) << 8 | u16::from(cpu.regs.f_),
        exp.af_
    );
    check!(
        "BC_",
        u16::from(cpu.regs.b_) << 8 | u16::from(cpu.regs.c_),
        exp.bc_
    );
    check!(
        "DE_",
        u16::from(cpu.regs.d_) << 8 | u16::from(cpu.regs.e_),
        exp.de_
    );
    check!(
        "HL_",
        u16::from(cpu.regs.h_) << 8 | u16::from(cpu.regs.l_),
        exp.hl_
    );
    check!("IX", cpu.regs.ix(), exp.ix);
    check!("IY", cpu.regs.iy(), exp.iy);
    check!("SP", cpu.regs.sp, exp.sp);
    check!("PC", cpu.regs.pc, exp.pc);
    check!("MP", cpu.regs.memptr, exp.memptr);
    if cpu.regs.i != exp.i {
        errs.push(format!("I: got {:02X} want {:02X}", cpu.regs.i, exp.i));
    }
    if cpu.regs.r != exp.r {
        errs.push(format!("R: got {:02X} want {:02X}", cpu.regs.r, exp.r));
    }
    if cpu.regs.iff1 != exp.iff1 || cpu.regs.iff2 != exp.iff2 {
        errs.push(format!(
            "IFF: got {}/{} want {}/{}",
            cpu.regs.iff1, cpu.regs.iff2, exp.iff1, exp.iff2
        ));
    }
    if cpu.regs.im != exp.im {
        errs.push(format!("IM: got {} want {}", cpu.regs.im, exp.im));
    }
    if cpu.regs.halted != exp.halted {
        errs.push(format!("halt: got {} want {}", cpu.regs.halted, exp.halted));
    }
    let dt = cpu.t - start;
    if dt != exp.tstates {
        errs.push(format!("T: got {dt} want {}", exp.tstates));
    }
    for (a, v) in &exp.mem {
        let got = bus.mem.data[*a as usize];
        if got != *v {
            errs.push(format!("mem[{a:04X}]: got {got:02X} want {v:02X}"));
        }
    }

    let got_events: Vec<FuseEvent> = cpu
        .fuse_log
        .take()
        .unwrap_or_default()
        .into_iter()
        .map(|e| FuseEvent {
            t: e.t.wrapping_sub(start),
            ..e
        })
        .collect();
    if got_events != exp.events {
        let mut diff = String::from("events mismatch:\n");
        let n = got_events.len().max(exp.events.len());
        for i in 0..n {
            let g = got_events.get(i).map(format_fuse_event);
            let w = exp.events.get(i).map(format_fuse_event);
            if g != w {
                diff.push_str(&format!(
                    "  [{i}] got {} want {}\n",
                    g.as_deref().unwrap_or("<missing>"),
                    w.as_deref().unwrap_or("<missing>")
                ));
            }
        }
        if got_events.len() != exp.events.len() {
            diff.push_str(&format!(
                "  (len got {} want {})\n",
                got_events.len(),
                exp.events.len()
            ));
        }
        errs.push(diff);
    }

    if errs.is_empty() {
        Ok(())
    } else {
        let dump = fuse_disasm_window(&tin.mem, tin.cpu.regs.pc, 8);
        Err(format!("{}: {}\n{dump}", tin.name, errs.join("; ")))
    }
}

/// Disassemble `count` instructions from Fuse test memory at `pc` (start PC).
fn fuse_disasm_window(mem: &FlatMem, pc: u16, count: usize) -> String {
    let count = count.clamp(1, 16);
    let mut out = format!("disasm @{pc:04X}:\n");
    let mut addr = pc;
    for _ in 0..count {
        let mut buf = [0u8; 4];
        for (i, b) in buf.iter_mut().enumerate() {
            *b = mem.data[addr.wrapping_add(i as u16) as usize];
        }
        let d = crate::disasm_one(&buf);
        let n = usize::from(d.len.max(1));
        out.push_str(&format!("{addr:04X}  "));
        for (i, b) in buf.iter().enumerate() {
            if i < n {
                out.push_str(&format!("{b:02X} "));
            } else {
                out.push_str("   ");
            }
        }
        out.push_str(&d.text);
        out.push('\n');
        addr = addr.wrapping_add(d.len.max(1) as u16);
    }
    out
}

#[test]
fn fuse_mismatch_includes_disasm_at_start_pc() {
    let dir = fixtures_dir();
    let tests = parse_in_file(&dir.join("tests.in"));
    let expected = parse_expected(&dir.join("tests.expected"));
    let (tin, exp) = tests
        .iter()
        .zip(expected.iter())
        .find(|(t, _)| t.name == "00")
        .expect("NOP test");
    let mut exp = exp.clone();
    exp.pc = exp.pc.wrapping_add(1);
    let err = run_case(tin, &exp).expect_err("forced PC mismatch");
    let start = tin.cpu.regs.pc;
    assert!(err.contains("PC:"), "{err}");
    assert!(err.contains(&format!("disasm @{start:04X}:")), "{err}");
    assert!(err.contains("NOP"), "{err}");
}

#[test]
fn fuse_smoke_nop() {
    let dir = fixtures_dir();
    let tests = parse_in_file(&dir.join("tests.in"));
    let expected = parse_expected(&dir.join("tests.expected"));
    assert!(!tests.is_empty());
    assert_eq!(tests.len(), expected.len());
    let (tin, exp) = tests
        .iter()
        .zip(expected.iter())
        .find(|(t, _)| t.name == "00")
        .expect("NOP test");
    run_case(tin, exp).unwrap_or_else(|e| panic!("{e}"));
}

#[test]
fn fuse_all_vectors() {
    let dir = fixtures_dir();
    let tests = parse_in_file(&dir.join("tests.in"));
    let expected = parse_expected(&dir.join("tests.expected"));
    assert_eq!(tests.len(), expected.len());
    let mut failed = 0usize;
    let mut first_errs = Vec::new();
    for (tin, exp) in tests.iter().zip(expected.iter()) {
        assert_eq!(tin.name, exp.name);
        if let Err(e) = run_case(tin, exp) {
            failed += 1;
            if first_errs.len() < 40 {
                first_errs.push(e);
            }
        }
    }
    if failed > 0 {
        panic!(
            "{failed}/{} Fuse tests failed. First errors:\n{}",
            tests.len(),
            first_errs.join("\n")
        );
    }
}

#[test]
fn dd_ld_ixh_smoke() {
    use crate::bus::FlatMem;
    use crate::cpu::Cpu;
    let mut cpu = Cpu::new();
    let mut mem = FlatMem::new();
    mem.data[0] = 0xdd;
    mem.data[1] = 0x26;
    mem.data[2] = 0xad;
    cpu.regs.set_ix(0x5f40);
    cpu.regs.set_hl(0xadea);
    cpu.step(&mut mem);
    assert_eq!(cpu.regs.ix(), 0xad40, "IX");
    assert_eq!(cpu.regs.hl(), 0xadea, "HL unchanged");
}
