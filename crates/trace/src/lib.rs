//! Structured emulator tracing: category-gated ring buffer with dump API.
//!
//! Hot path: when no categories are enabled, [`emit`] / [`enabled`] are a single
//! `AtomicU64` load (`Relaxed`) and return — no allocation and no lock.

#![allow(clippy::pedantic)]

use std::collections::VecDeque;
use std::fmt::{Display, Formatter, Result as FmtResult};
use std::fs::File;
use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

/// Default ring capacity (overridable via `SPEC_CHUM_TRACE_CAPACITY`).
pub const DEFAULT_CAPACITY: usize = 8192;

/// Trace categories (bitflags). Combine with `|`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Category(u64);

impl Category {
    pub const NONE: Self = Self(0);
    pub const CPU: Self = Self(1 << 0);
    pub const BUS: Self = Self(1 << 1);
    pub const TAPE: Self = Self(1 << 2);
    pub const ULA: Self = Self(1 << 3);
    pub const MACHINE: Self = Self(1 << 4);
    pub const AY: Self = Self(1 << 5);
    pub const DISK: Self = Self(1 << 6);
    pub const MEM: Self = Self(1 << 7);
    /// Convenience: BUS|TAPE|ULA|MACHINE (excludes high-volume CPU, AY, DISK, MEM).
    pub const DEFAULT: Self = Self(Self::BUS.0 | Self::TAPE.0 | Self::ULA.0 | Self::MACHINE.0);
    pub const ALL: Self = Self(
        Self::CPU.0
            | Self::BUS.0
            | Self::TAPE.0
            | Self::ULA.0
            | Self::MACHINE.0
            | Self::AY.0
            | Self::DISK.0
            | Self::MEM.0,
    );

    #[must_use]
    pub const fn bits(self) -> u64 {
        self.0
    }

    #[must_use]
    pub const fn from_bits(bits: u64) -> Self {
        Self(bits)
    }

    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    #[must_use]
    pub fn parse_list(s: &str) -> Self {
        let mut c = Self::NONE;
        for part in s.split(|ch: char| ch == ',' || ch.is_whitespace()) {
            let p = part.trim().to_ascii_lowercase();
            if p.is_empty() {
                continue;
            }
            c = c.union(match p.as_str() {
                "all" => Self::ALL,
                "default" | "debug" => Self::DEFAULT,
                "cpu" | "z80" => Self::CPU,
                "bus" | "io" => Self::BUS,
                "tape" => Self::TAPE,
                "ula" | "video" => Self::ULA,
                "machine" | "mach" => Self::MACHINE,
                "ay" | "psg" => Self::AY,
                "disk" | "fdc" => Self::DISK,
                "mem" | "memory" => Self::MEM,
                _ => Self::NONE,
            });
        }
        c
    }
}

impl std::ops::BitOr for Category {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        self.union(rhs)
    }
}

impl std::ops::BitOrAssign for Category {
    fn bitor_assign(&mut self, rhs: Self) {
        *self = self.union(rhs);
    }
}

/// Why a flash-load attempt skipped or failed a TAP block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum FlashSkipReason {
    Paused = 0,
    NoBlock = 1,
    EmptyBlock = 2,
    WrongFlag = 3,
    LengthMismatch = 4,
    ChecksumFail = 5,
}

impl Display for FlashSkipReason {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        f.write_str(match self {
            Self::Paused => "paused",
            Self::NoBlock => "no_block",
            Self::EmptyBlock => "empty_block",
            Self::WrongFlag => "wrong_flag",
            Self::LengthMismatch => "length_mismatch",
            Self::ChecksumFail => "checksum_fail",
        })
    }
}

/// Compact Z80 register snapshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RegSnap {
    pub pc: u16,
    pub sp: u16,
    pub af: u16,
    pub bc: u16,
    pub de: u16,
    pub hl: u16,
    pub ix: u16,
    pub iy: u16,
    pub af_: u16,
    pub bc_: u16,
    pub de_: u16,
    pub hl_: u16,
    pub i: u8,
    pub r: u8,
    pub im: u8,
    pub memptr: u16,
    pub iff1: bool,
    pub iff2: bool,
    pub halted: bool,
}

impl Display for RegSnap {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        write!(
            f,
            "PC={:04X} SP={:04X} AF={:04X} BC={:04X} DE={:04X} HL={:04X} IX={:04X} IY={:04X} AF'={:04X} I={:02X} R={:02X} IM={} IFF1={} HALT={}",
            self.pc,
            self.sp,
            self.af,
            self.bc,
            self.de,
            self.hl,
            self.ix,
            self.iy,
            self.af_,
            self.i,
            self.r,
            self.im,
            u8::from(self.iff1),
            u8::from(self.halted)
        )
    }
}

/// Event payload (stack-friendly; no heap).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventKind {
    CpuStep {
        pc: u16,
        bytes: [u8; 4],
        len: u8,
        dt: u16,
        regs: RegSnap,
    },
    CpuIrq {
        pc: u16,
        im: u8,
    },
    CpuHalt {
        pc: u16,
    },
    BusPortFe {
        write: bool,
        value: u8,
        ear: bool,
    },
    BusPort7ffd {
        value: u8,
    },
    BusPort1ffd {
        value: u8,
    },
    BusContend {
        addr: u16,
        frame_t: u32,
        wait: u32,
    },
    BusFloating {
        port: u16,
        frame_t: u32,
        value: u8,
    },
    AySelect {
        reg: u8,
    },
    AyWrite {
        reg: u8,
        value: u8,
    },
    DiskFdc {
        port: u16,
        write: bool,
        value: u8,
    },
    MemWatch {
        addr: u16,
        write: bool,
        value: u8,
    },
    TapePlay {
        block: u32,
        blocks: u32,
    },
    TapePause {
        block: u32,
    },
    TapeRewind,
    TapeBlock {
        index: u32,
        flag: u8,
        len: u16,
    },
    FlashLoadEnter {
        regs: RegSnap,
        flag_expected: u8,
        load: bool,
        addr: u16,
        len: u16,
        block: u32,
    },
    FlashLoadExit {
        success: bool,
        bytes: u16,
        block_after: u32,
        regs: RegSnap,
    },
    FlashLoadSkip {
        reason: FlashSkipReason,
        block: u32,
        flag_got: u8,
        flag_want: u8,
        block_len: u16,
        want_len: u16,
    },
    TapeEarRate {
        edges_per_frame: u32,
        level: bool,
    },
    UlaFrame {
        frame: u32,
    },
    UlaInt {
        frame_t: u32,
    },
    UlaBorder {
        color: u8,
        frame_t: u32,
    },
    MachineModel {
        model: u8,
    },
    MachineLoadMode {
        flash_load: bool,
        speed: u8,
    },
    MachineLdBytesHold {
        holding: bool,
        pc: u16,
    },
    MachineSnapshot {
        pc: u16,
        sp: u16,
        border: u8,
    },
}

impl EventKind {
    #[must_use]
    pub fn category(self) -> Category {
        match self {
            Self::CpuStep { .. } | Self::CpuIrq { .. } | Self::CpuHalt { .. } => Category::CPU,
            Self::BusPortFe { .. }
            | Self::BusPort7ffd { .. }
            | Self::BusPort1ffd { .. }
            | Self::BusContend { .. }
            | Self::BusFloating { .. } => Category::BUS,
            Self::AySelect { .. } | Self::AyWrite { .. } => Category::AY,
            Self::DiskFdc { .. } => Category::DISK,
            Self::MemWatch { .. } => Category::MEM,
            Self::TapePlay { .. }
            | Self::TapePause { .. }
            | Self::TapeRewind
            | Self::TapeBlock { .. }
            | Self::FlashLoadEnter { .. }
            | Self::FlashLoadExit { .. }
            | Self::FlashLoadSkip { .. }
            | Self::TapeEarRate { .. } => Category::TAPE,
            Self::UlaFrame { .. } | Self::UlaInt { .. } | Self::UlaBorder { .. } => Category::ULA,
            Self::MachineModel { .. }
            | Self::MachineLoadMode { .. }
            | Self::MachineLdBytesHold { .. }
            | Self::MachineSnapshot { .. } => Category::MACHINE,
        }
    }

    #[must_use]
    pub fn pc(self) -> Option<u16> {
        match self {
            Self::CpuStep { pc, .. }
            | Self::CpuIrq { pc, .. }
            | Self::CpuHalt { pc }
            | Self::MachineLdBytesHold { pc, .. }
            | Self::MachineSnapshot { pc, .. } => Some(pc),
            Self::FlashLoadEnter { regs, .. } | Self::FlashLoadExit { regs, .. } => Some(regs.pc),
            _ => None,
        }
    }
}

impl Display for EventKind {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        match *self {
            Self::CpuStep {
                pc,
                bytes,
                len,
                dt,
                regs,
            } => {
                let n = usize::from(len.clamp(1, 4));
                write!(f, "cpu.step pc={pc:04X} dt={dt} bytes=")?;
                for (i, b) in bytes.iter().take(n).enumerate() {
                    if i > 0 {
                        write!(f, " ")?;
                    }
                    write!(f, "{b:02X}")?;
                }
                write!(f, " {regs}")
            }
            Self::CpuIrq { pc, im } => write!(f, "cpu.irq pc={pc:04X} im={im}"),
            Self::CpuHalt { pc } => write!(f, "cpu.halt pc={pc:04X}"),
            Self::BusPortFe { write, value, ear } => write!(
                f,
                "bus.fe {} val={value:02X} ear={}",
                if write { "out" } else { "in" },
                u8::from(ear)
            ),
            Self::BusPort7ffd { value } => write!(f, "bus.7ffd out={value:02X}"),
            Self::BusPort1ffd { value } => write!(f, "bus.1ffd out={value:02X}"),
            Self::BusContend {
                addr,
                frame_t,
                wait,
            } => write!(f, "bus.contend addr={addr:04X} frame_t={frame_t} wait={wait}"),
            Self::BusFloating {
                port,
                frame_t,
                value,
            } => write!(f, "bus.floating port={port:04X} frame_t={frame_t} val={value:02X}"),
            Self::AySelect { reg } => write!(f, "ay.select reg={reg}"),
            Self::AyWrite { reg, value } => write!(f, "ay.write reg={reg} val={value:02X}"),
            Self::DiskFdc { port, write, value } => write!(
                f,
                "disk.fdc {} port={port:04X} val={value:02X}",
                if write { "out" } else { "in" }
            ),
            Self::MemWatch { addr, write, value } => write!(
                f,
                "mem.watch {} addr={addr:04X} val={value:02X}",
                if write { "wr" } else { "rd" }
            ),
            Self::TapePlay { block, blocks } => write!(f, "tape.play block={block}/{blocks}"),
            Self::TapePause { block } => write!(f, "tape.pause block={block}"),
            Self::TapeRewind => write!(f, "tape.rewind"),
            Self::TapeBlock { index, flag, len } => {
                write!(f, "tape.block idx={index} flag={flag:02X} len={len}")
            }
            Self::FlashLoadEnter {
                regs,
                flag_expected,
                load,
                addr,
                len,
                block,
            } => write!(
                f,
                "tape.flash.enter block={block} flag={flag_expected:02X} load={} dest={addr:04X} len={len} {regs}",
                u8::from(load)
            ),
            Self::FlashLoadExit {
                success,
                bytes,
                block_after,
                regs,
            } => write!(
                f,
                "tape.flash.exit ok={} bytes={bytes} block_after={block_after} {regs}",
                u8::from(success)
            ),
            Self::FlashLoadSkip {
                reason,
                block,
                flag_got,
                flag_want,
                block_len,
                want_len,
            } => write!(
                f,
                "tape.flash.skip reason={reason} block={block} flag_got={flag_got:02X} flag_want={flag_want:02X} block_len={block_len} want_len={want_len}"
            ),
            Self::TapeEarRate {
                edges_per_frame,
                level,
            } => write!(
                f,
                "tape.ear_rate window_edges={edges_per_frame} level={}",
                u8::from(level)
            ),
            Self::UlaFrame { frame } => write!(f, "ula.frame n={frame}"),
            Self::UlaInt { frame_t } => write!(f, "ula.int frame_t={frame_t}"),
            Self::UlaBorder { color, frame_t } => {
                write!(f, "ula.border color={color} frame_t={frame_t}")
            }
            Self::MachineModel { model } => write!(f, "machine.model id={model}"),
            Self::MachineLoadMode { flash_load, speed } => write!(
                f,
                "machine.load_mode flash={} speed={speed}x",
                u8::from(flash_load)
            ),
            Self::MachineLdBytesHold { holding, pc } => write!(
                f,
                "machine.ld_bytes_hold holding={} pc={pc:04X}",
                u8::from(holding)
            ),
            Self::MachineSnapshot { pc, sp, border } => {
                write!(f, "machine.snapshot pc={pc:04X} sp={sp:04X} border={border}")
            }
        }
    }
}

/// One ring entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TraceEvent {
    pub seq: u64,
    /// Absolute CPU T-state when known; otherwise 0.
    pub t: u64,
    pub kind: EventKind,
}

impl Display for TraceEvent {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        write!(f, "#{:<6} t={:<12} {}", self.seq, self.t, self.kind)
    }
}

struct Ring {
    capacity: usize,
    events: VecDeque<TraceEvent>,
    next_seq: u64,
}

impl Ring {
    fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            events: VecDeque::with_capacity(capacity.min(4096)),
            next_seq: 1,
        }
    }

    fn push(&mut self, t: u64, kind: EventKind) -> TraceEvent {
        let ev = TraceEvent {
            seq: self.next_seq,
            t,
            kind,
        };
        self.next_seq = self.next_seq.wrapping_add(1);
        if self.events.len() >= self.capacity {
            self.events.pop_front();
        }
        self.events.push_back(ev);
        ev
    }

    fn clear(&mut self) {
        self.events.clear();
    }

    fn snapshot(&self) -> Vec<TraceEvent> {
        self.events.iter().copied().collect()
    }
}

static ENABLED: AtomicU64 = AtomicU64::new(0);
static CPU_EVERY: AtomicU32 = AtomicU32::new(1);
static CPU_COUNTER: AtomicU64 = AtomicU64::new(0);
static T_HINT: AtomicU64 = AtomicU64::new(0);
static RING: OnceLock<Mutex<Ring>> = OnceLock::new();
static ENV_INIT: OnceLock<()> = OnceLock::new();

fn ring() -> &'static Mutex<Ring> {
    RING.get_or_init(|| {
        let cap = std::env::var("SPEC_CHUM_TRACE_CAPACITY")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(DEFAULT_CAPACITY);
        Mutex::new(Ring::new(cap))
    })
}

/// Apply `SPEC_CHUM_DEBUG` / `SPEC_CHUM_TRACE` once (idempotent).
pub fn init_from_env() {
    ENV_INIT.get_or_init(|| {
        if let Ok(every) = std::env::var("SPEC_CHUM_TRACE_CPU_EVERY") {
            if let Ok(n) = every.parse::<u32>() {
                CPU_EVERY.store(n.max(1), Ordering::Relaxed);
            }
        }
        if let Ok(v) = std::env::var("SPEC_CHUM_TRACE") {
            let c = Category::parse_list(&v);
            if c.bits() != 0 {
                enable(c);
                return;
            }
        }
        if let Ok(v) = std::env::var("SPEC_CHUM_DEBUG") {
            let t = v.trim();
            if t == "1" || t.eq_ignore_ascii_case("true") || t.eq_ignore_ascii_case("yes") {
                enable(Category::DEFAULT);
            } else if !t.is_empty() && t != "0" && !t.eq_ignore_ascii_case("false") {
                enable(Category::parse_list(t));
            }
        }
    });
}

/// Replace enabled categories (does not clear the ring).
pub fn enable(cats: Category) {
    ENABLED.store(cats.bits(), Ordering::Relaxed);
}

/// Add categories without clearing existing ones.
pub fn enable_add(cats: Category) {
    ENABLED.fetch_or(cats.bits(), Ordering::Relaxed);
}

/// Disable all tracing (keeps ring contents).
pub fn disable() {
    ENABLED.store(0, Ordering::Relaxed);
}

#[must_use]
pub fn categories() -> Category {
    Category(ENABLED.load(Ordering::Relaxed))
}

#[inline]
#[must_use]
pub fn enabled(cat: Category) -> bool {
    ENABLED.load(Ordering::Relaxed) & cat.bits() != 0
}

#[inline]
#[must_use]
pub fn any_enabled() -> bool {
    ENABLED.load(Ordering::Relaxed) != 0
}

/// Hint used as `t` for subsequent events when not passed explicitly.
pub fn set_t_hint(t: u64) {
    T_HINT.store(t, Ordering::Relaxed);
}

#[must_use]
pub fn t_hint() -> u64 {
    T_HINT.load(Ordering::Relaxed)
}

/// Emit when `kind.category()` is enabled. No-op (no lock) when disabled.
#[inline]
pub fn emit(kind: EventKind) {
    let cat = kind.category();
    if !enabled(cat) {
        return;
    }
    if matches!(kind, EventKind::CpuStep { .. }) {
        let every = CPU_EVERY.load(Ordering::Relaxed).max(1);
        let n = CPU_COUNTER.fetch_add(1, Ordering::Relaxed);
        if !n.is_multiple_of(u64::from(every)) {
            return;
        }
    }
    let t = T_HINT.load(Ordering::Relaxed);
    if let Ok(mut g) = ring().lock() {
        let ev = g.push(t, kind);
        drop(g);
        maybe_append(&ev);
    }
}

/// Emit with an explicit T-state timestamp.
#[inline]
pub fn emit_at(t: u64, kind: EventKind) {
    let cat = kind.category();
    if !enabled(cat) {
        return;
    }
    if matches!(kind, EventKind::CpuStep { .. }) {
        let every = CPU_EVERY.load(Ordering::Relaxed).max(1);
        let n = CPU_COUNTER.fetch_add(1, Ordering::Relaxed);
        if !n.is_multiple_of(u64::from(every)) {
            return;
        }
    }
    T_HINT.store(t, Ordering::Relaxed);
    if let Ok(mut g) = ring().lock() {
        let ev = g.push(t, kind);
        drop(g);
        maybe_append(&ev);
    }
}

/// Clear the ring (does not change enabled categories).
pub fn clear() {
    if let Ok(mut g) = ring().lock() {
        g.clear();
    }
    CPU_COUNTER.store(0, Ordering::Relaxed);
}

/// Snapshot of ring contents (oldest → newest).
#[must_use]
pub fn snapshot() -> Vec<TraceEvent> {
    ring().lock().map(|g| g.snapshot()).unwrap_or_default()
}

/// How many events are currently buffered.
#[must_use]
pub fn len() -> usize {
    ring().lock().map(|g| g.events.len()).unwrap_or(0)
}

fn describe_categories(c: Category) -> String {
    let mut parts = Vec::new();
    if c.contains(Category::CPU) {
        parts.push("cpu");
    }
    if c.contains(Category::BUS) {
        parts.push("bus");
    }
    if c.contains(Category::TAPE) {
        parts.push("tape");
    }
    if c.contains(Category::ULA) {
        parts.push("ula");
    }
    if c.contains(Category::MACHINE) {
        parts.push("machine");
    }
    if c.contains(Category::AY) {
        parts.push("ay");
    }
    if c.contains(Category::DISK) {
        parts.push("disk");
    }
    if c.contains(Category::MEM) {
        parts.push("mem");
    }
    if parts.is_empty() {
        "none".into()
    } else {
        parts.join(",")
    }
}

/// Format the ring as text (one event per line) with a short header.
#[must_use]
pub fn dump_string() -> String {
    let cats = categories();
    let events = snapshot();
    let mut out = String::with_capacity(events.len().saturating_mul(96) + 128);
    out.push_str(&format!(
        "# spec_chum trace dump events={} categories=0x{:x} ({})\n",
        events.len(),
        cats.bits(),
        describe_categories(cats)
    ));
    for ev in &events {
        out.push_str(&ev.to_string());
        out.push('\n');
    }
    out
}

/// Optional filters for [`dump_filtered`].
#[derive(Clone, Copy, Debug, Default)]
pub struct DumpFilter {
    /// If non-zero, keep events whose category intersects this mask.
    pub category: Category,
    pub t_min: Option<u64>,
    pub t_max: Option<u64>,
    pub pc_min: Option<u16>,
    pub pc_max: Option<u16>,
    pub last_n: Option<usize>,
}

#[must_use]
pub fn dump_filtered(filter: DumpFilter) -> String {
    let mut events = snapshot();
    if filter.category.bits() != 0 {
        events.retain(|e| e.kind.category().bits() & filter.category.bits() != 0);
    }
    if let Some(t0) = filter.t_min {
        events.retain(|e| e.t >= t0);
    }
    if let Some(t1) = filter.t_max {
        events.retain(|e| e.t <= t1);
    }
    if filter.pc_min.is_some() || filter.pc_max.is_some() {
        events.retain(|e| {
            let Some(pc) = e.kind.pc() else {
                return false;
            };
            if filter.pc_min.is_some_and(|lo| pc < lo) {
                return false;
            }
            if filter.pc_max.is_some_and(|hi| pc > hi) {
                return false;
            }
            true
        });
    }
    if let Some(n) = filter.last_n {
        let skip = events.len().saturating_sub(n);
        events.drain(..skip);
    }
    let mut out = String::with_capacity(events.len().saturating_mul(96) + 64);
    out.push_str(&format!(
        "# spec_chum trace dump events={} (filtered)\n",
        events.len()
    ));
    for ev in &events {
        out.push_str(&ev.to_string());
        out.push('\n');
    }
    out
}

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", u32::from(c))),
            c => out.push(c),
        }
    }
    out
}

fn event_json(ev: &TraceEvent) -> String {
    let cat = describe_categories(ev.kind.category());
    let text = json_escape(&ev.kind.to_string());
    let pc = ev.kind.pc().map_or("null".into(), |p| p.to_string());
    format!(
        "{{\"seq\":{},\"t\":{},\"cat\":\"{cat}\",\"pc\":{pc},\"text\":\"{text}\"}}",
        ev.seq, ev.t
    )
}

/// JSON array of ring events (hand-rolled, no serde).
#[must_use]
pub fn dump_json() -> String {
    let events = snapshot();
    let mut out = String::from("[\n");
    for (i, ev) in events.iter().enumerate() {
        if i > 0 {
            out.push_str(",\n");
        }
        out.push_str(&event_json(ev));
    }
    out.push_str("\n]\n");
    out
}

/// One JSON object per line.
#[must_use]
pub fn dump_ndjson() -> String {
    let mut out = String::new();
    for ev in snapshot() {
        out.push_str(&event_json(&ev));
        out.push('\n');
    }
    out
}

fn maybe_append(ev: &TraceEvent) {
    static SINK: OnceLock<Option<Mutex<std::io::BufWriter<File>>>> = OnceLock::new();
    let sink = SINK.get_or_init(|| {
        let flag = std::env::var("SPEC_CHUM_TRACE_APPEND")
            .map(|v| {
                let t = v.trim();
                t == "1" || t.eq_ignore_ascii_case("true") || t.eq_ignore_ascii_case("yes")
            })
            .unwrap_or(false);
        if !flag {
            return None;
        }
        let path = std::env::var("SPEC_CHUM_TRACE_FILE").ok()?;
        if path.is_empty() {
            return None;
        }
        let f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .ok()?;
        Some(Mutex::new(std::io::BufWriter::new(f)))
    });
    let Some(lock) = sink.as_ref() else {
        return;
    };
    if let Ok(mut w) = lock.lock() {
        let _ = writeln!(w, "{ev}");
    }
}

/// Write dump to `w`.
pub fn dump_to_writer(mut w: impl Write) -> io::Result<()> {
    w.write_all(dump_string().as_bytes())
}

/// Write dump to a file path (creates/truncates).
pub fn dump_to_file(path: impl AsRef<Path>) -> io::Result<()> {
    let mut f = File::create(path)?;
    dump_to_writer(&mut f)
}

/// Dump to stderr (agents / CI failure path).
pub fn dump_to_stderr() {
    let _ = dump_to_writer(io::stderr());
}

/// If `SPEC_CHUM_TRACE_FILE` is set, write the dump there.
pub fn dump_to_env_file() -> io::Result<Option<std::path::PathBuf>> {
    let Ok(path) = std::env::var("SPEC_CHUM_TRACE_FILE") else {
        return Ok(None);
    };
    if path.is_empty() {
        return Ok(None);
    }
    let p = std::path::PathBuf::from(path);
    dump_to_file(&p)?;
    Ok(Some(p))
}

/// Exclusive lock for tests that mutate the global ring (avoids cross-test races).
pub fn test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// Test helper: enable categories, clear ring, run `f`, restore previous enable mask.
pub fn with_trace<R>(cats: Category, f: impl FnOnce() -> R) -> R {
    struct Restore(Category);
    impl Drop for Restore {
        fn drop(&mut self) {
            enable(self.0);
        }
    }
    let _restore = Restore(categories());
    clear();
    enable(cats);
    f()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_emit_is_noop() {
        let _g = test_lock();
        disable();
        clear();
        emit(EventKind::TapeRewind);
        assert_eq!(len(), 0);
    }

    #[test]
    fn ring_keeps_last_n() {
        let _g = test_lock();
        disable();
        clear();
        enable(Category::TAPE);
        {
            let mut g = ring().lock().expect("lock");
            *g = Ring::new(4);
        }
        for i in 0..10u32 {
            emit(EventKind::TapePause { block: i });
        }
        let snap = snapshot();
        assert_eq!(snap.len(), 4);
        assert_eq!(snap[0].kind, EventKind::TapePause { block: 6 });
        assert_eq!(snap[3].kind, EventKind::TapePause { block: 9 });
        {
            let mut g = ring().lock().expect("lock");
            *g = Ring::new(DEFAULT_CAPACITY);
        }
        disable();
        clear();
    }

    #[test]
    fn parse_categories() {
        assert!(Category::parse_list("tape,cpu").contains(Category::TAPE));
        assert!(Category::parse_list("tape,cpu").contains(Category::CPU));
        assert!(Category::parse_list("all").contains(Category::ULA));
        assert_eq!(
            Category::parse_list("default").bits(),
            Category::DEFAULT.bits()
        );
        assert_eq!(Category::default(), Category::NONE);
    }

    #[test]
    fn dump_contains_flash_skip() {
        let _g = test_lock();
        disable();
        clear();
        enable(Category::TAPE);
        emit(EventKind::FlashLoadSkip {
            reason: FlashSkipReason::WrongFlag,
            block: 0,
            flag_got: 0xff,
            flag_want: 0x00,
            block_len: 19,
            want_len: 17,
        });
        let s = dump_string();
        assert!(s.contains("tape.flash.skip"));
        assert!(s.contains("wrong_flag"));
        disable();
        clear();
    }
}
