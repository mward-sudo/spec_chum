//! Trace event payloads and display formatting.

use std::fmt::{Display, Formatter, Result as FmtResult};

use crate::category::Category;

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
        experience_load: bool,
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
            Self::MachineLoadMode { flash_load, speed, experience_load } => write!(
                f,
                "machine.load_mode flash={} speed={speed}x experience={}",
                u8::from(flash_load),
                u8::from(experience_load)
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
