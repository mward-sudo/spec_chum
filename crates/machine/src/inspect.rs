//! Machine inspect snapshot for agents, CLI, and debugger UIs.

use std::fmt::{Display, Formatter, Result as FmtResult};

use ula::{
    floating_bus_byte_128, floating_bus_byte_48, int_active_48, FRAME_TSTATES_128,
    FRAME_TSTATES_48, INT_LENGTH_128, INT_LENGTH_48, LINES_128, LINES_48, T_LINE_128, T_LINE_48,
};
use z80::{disasm_one, Registers};

use crate::{Machine, Model};

/// Full architectural + ULA/bus snapshot (no framebuffer).
#[derive(Clone, Debug)]
pub struct Inspect {
    pub model: Model,
    pub regs: Registers,
    pub cpu_t: u64,
    pub frame_t: u32,
    pub t_line: u32,
    pub lines: u32,
    pub frame_tstates: u32,
    pub raster_line: u32,
    pub raster_x: u32,
    pub int_active: bool,
    pub int_length: u32,
    pub contend_at_pc: u32,
    pub floating_bus: Option<u8>,
    pub border: u8,
    pub ear: bool,
    pub mic: bool,
    pub beeper: bool,
    pub paging: Paging,
    pub tape: Option<TapeInspect>,
    pub ay_regs: Option<[u8; 16]>,
    pub ay_selected: Option<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Paging {
    pub page_7ffd: Option<u8>,
    pub page_1ffd: Option<u8>,
    pub rom_bank: u8,
    pub ram_c000: Option<u8>,
    pub screen_bank: Option<u8>,
    pub special: bool,
    pub locked: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TapeInspect {
    pub playing: bool,
    pub flash_load: bool,
    pub speed: u32,
    pub block_index: u32,
    pub block_count: u32,
}

impl Machine {
    #[must_use]
    pub fn frame_t(&self) -> u32 {
        match self {
            Self::Spec48 { bus, .. } => bus.frame_t,
            Self::Spec128 { bus, .. } => bus.frame_t,
            Self::SpecPlus3 { bus, .. } => bus.frame_t,
        }
    }

    #[must_use]
    pub fn inspect(&self) -> Inspect {
        let regs = self.cpu().regs;
        let cpu_t = self.cpu().t;
        let pc = regs.pc;
        match self {
            Self::Spec48 {
                bus,
                tape,
                tape_opts,
                ..
            } => {
                let frame_t = bus.frame_t;
                Inspect {
                    model: Model::Spectrum48,
                    regs,
                    cpu_t,
                    frame_t,
                    t_line: T_LINE_48,
                    lines: LINES_48,
                    frame_tstates: FRAME_TSTATES_48,
                    raster_line: frame_t / T_LINE_48,
                    raster_x: frame_t % T_LINE_48,
                    int_active: int_active_48(frame_t),
                    int_length: INT_LENGTH_48,
                    contend_at_pc: bus.contend_at(pc),
                    floating_bus: floating_bus_byte_48(frame_t, bus.screen_bytes()),
                    border: bus.border,
                    ear: bus.ear,
                    mic: bus.mic,
                    beeper: bus.beeper,
                    paging: Paging {
                        page_7ffd: None,
                        page_1ffd: None,
                        rom_bank: 0,
                        ram_c000: None,
                        screen_bank: None,
                        special: false,
                        locked: false,
                    },
                    tape: tape.as_ref().map(|t| TapeInspect {
                        playing: t.playing(),
                        flash_load: tape_opts.flash_load,
                        speed: tape_opts.speed,
                        block_index: t.block().unwrap_or(0) as u32,
                        block_count: t.block_count() as u32,
                    }),
                    ay_regs: None,
                    ay_selected: None,
                }
            }
            Self::Spec128 {
                bus,
                tape,
                tape_opts,
                ..
            } => {
                let frame_t = bus.frame_t;
                Inspect {
                    model: Model::Spectrum128,
                    regs,
                    cpu_t,
                    frame_t,
                    t_line: T_LINE_128,
                    lines: LINES_128,
                    frame_tstates: FRAME_TSTATES_128,
                    raster_line: frame_t / T_LINE_128,
                    raster_x: frame_t % T_LINE_128,
                    int_active: frame_t < INT_LENGTH_128,
                    int_length: INT_LENGTH_128,
                    contend_at_pc: bus.contend_at(pc),
                    floating_bus: floating_bus_byte_128(frame_t, bus.screen_bytes()),
                    border: bus.border,
                    ear: bus.ear,
                    mic: false,
                    beeper: bus.beeper,
                    paging: Paging {
                        page_7ffd: Some(bus.page),
                        page_1ffd: None,
                        rom_bank: u8::from(bus.page & 0x10 != 0),
                        ram_c000: Some(bus.page & 7),
                        screen_bank: Some(if bus.page & 0x08 != 0 { 7 } else { 5 }),
                        special: false,
                        locked: bus.locked,
                    },
                    tape: tape.as_ref().map(|t| TapeInspect {
                        playing: t.playing(),
                        flash_load: tape_opts.flash_load,
                        speed: tape_opts.speed,
                        block_index: t.block().unwrap_or(0) as u32,
                        block_count: t.block_count() as u32,
                    }),
                    ay_regs: Some(bus.ay.regs),
                    ay_selected: Some(bus.ay.selected),
                }
            }
            Self::SpecPlus3 {
                bus,
                tape,
                tape_opts,
                ..
            } => {
                let frame_t = bus.frame_t;
                Inspect {
                    model: if bus.disk_interface {
                        Model::SpectrumPlus3
                    } else {
                        Model::SpectrumPlus2A
                    },
                    regs,
                    cpu_t,
                    frame_t,
                    t_line: T_LINE_128,
                    lines: LINES_128,
                    frame_tstates: FRAME_TSTATES_128,
                    raster_line: frame_t / T_LINE_128,
                    raster_x: frame_t % T_LINE_128,
                    int_active: frame_t < INT_LENGTH_128,
                    int_length: INT_LENGTH_128,
                    contend_at_pc: bus.contend_at(pc),
                    floating_bus: None,
                    border: bus.border,
                    ear: bus.ear,
                    mic: false,
                    beeper: bus.beeper,
                    paging: Paging {
                        page_7ffd: Some(bus.page_7ffd),
                        page_1ffd: Some(bus.page_1ffd),
                        rom_bank: bus.rom_num() as u8,
                        ram_c000: Some(bus.page_7ffd & 7),
                        screen_bank: Some(if bus.page_7ffd & 0x08 != 0 { 7 } else { 5 }),
                        special: bus.special_paging(),
                        locked: bus.locked,
                    },
                    tape: tape.as_ref().map(|t| TapeInspect {
                        playing: t.playing(),
                        flash_load: tape_opts.flash_load,
                        speed: tape_opts.speed,
                        block_index: t.block().unwrap_or(0) as u32,
                        block_count: t.block_count() as u32,
                    }),
                    ay_regs: Some(bus.ay.regs),
                    ay_selected: Some(bus.ay.selected),
                }
            }
        }
    }

    #[must_use]
    pub fn hexdump(&self, addr: u16, len: u16) -> String {
        let len = len.clamp(1, 4096);
        let mut out = String::new();
        let mut a = addr;
        let mut remaining = len;
        while remaining > 0 {
            let row = remaining.min(16);
            out.push_str(&format!("{a:04X}  "));
            let mut ascii = String::new();
            for i in 0..16 {
                if i < row {
                    let b = self.read_mem(a.wrapping_add(i));
                    out.push_str(&format!("{b:02X} "));
                    ascii.push(if (0x20..=0x7e).contains(&b) {
                        b as char
                    } else {
                        '.'
                    });
                } else {
                    out.push_str("   ");
                }
            }
            out.push_str(" |");
            out.push_str(&ascii);
            out.push_str("|\n");
            a = a.wrapping_add(row);
            remaining -= row;
        }
        out
    }

    #[must_use]
    pub fn disasm_window(&self, addr: u16, count: usize) -> String {
        let count = count.clamp(1, 64);
        let mut out = String::new();
        let mut pc = addr;
        for _ in 0..count {
            let mut buf = [0u8; 4];
            for (i, b) in buf.iter_mut().enumerate() {
                *b = self.read_mem(pc.wrapping_add(i as u16));
            }
            let d = disasm_one(&buf);
            let n = usize::from(d.len.max(1));
            out.push_str(&format!("{pc:04X}  "));
            for (i, b) in buf.iter().enumerate() {
                if i < n {
                    out.push_str(&format!("{b:02X} "));
                } else {
                    out.push_str("   ");
                }
            }
            out.push_str(&d.text);
            out.push('\n');
            pc = pc.wrapping_add(d.len as u16);
        }
        out
    }

    #[must_use]
    pub fn stack_words(&self, n: usize) -> Vec<u16> {
        let n = n.min(32);
        let sp = self.cpu().regs.sp;
        (0..n)
            .map(|i| {
                let a = sp.wrapping_add((i * 2) as u16);
                let lo = self.read_mem(a);
                let hi = self.read_mem(a.wrapping_add(1));
                u16::from(lo) | (u16::from(hi) << 8)
            })
            .collect()
    }
}

fn tape_json(t: &TapeInspect) -> String {
    format!(
        "{{\"playing\":{},\"flash_load\":{},\"speed\":{},\"block\":{},\"blocks\":{}}}",
        u8::from(t.playing),
        u8::from(t.flash_load),
        t.speed,
        t.block_index,
        t.block_count
    )
}

impl Inspect {
    /// Hand-rolled JSON (no serde).
    #[must_use]
    pub fn to_json(&self) -> String {
        let r = &self.regs;
        let model = match self.model {
            Model::Spectrum48 => "48k",
            Model::Spectrum128 => "128k",
            Model::SpectrumPlus2A => "plus2a",
            Model::SpectrumPlus3 => "plus3",
        };
        let tape = self.tape.as_ref().map_or("null".into(), tape_json);
        let ay = self.ay_regs.map_or("null".into(), |regs| {
            let list: Vec<String> = regs.iter().map(|b| format!("{b}")).collect();
            format!(
                "{{\"selected\":{},\"regs\":[{}]}}",
                self.ay_selected.unwrap_or(0),
                list.join(",")
            )
        });
        let fb = self.floating_bus.map_or("null".into(), |v| format!("{v}"));
        format!(
            "{{\
\"model\":\"{model}\",\
\"t\":{},\
\"frame_t\":{},\
\"line\":{},\
\"x\":{},\
\"int\":{},\
\"contend_pc\":{},\
\"floating_bus\":{fb},\
\"border\":{},\
\"ear\":{},\
\"beeper\":{},\
\"pc\":{},\"sp\":{},\"af\":{},\"bc\":{},\"de\":{},\"hl\":{},\
\"ix\":{},\"iy\":{},\"af_\":{},\"bc_\":{},\"de_\":{},\"hl_\":{},\
\"i\":{},\"r\":{},\"im\":{},\"memptr\":{},\"iff1\":{},\"iff2\":{},\"halted\":{},\
\"page_7ffd\":{},\"page_1ffd\":{},\"rom\":{},\"ram_c000\":{},\"screen\":{},\
\"tape\":{tape},\"ay\":{ay}\
}}",
            self.cpu_t,
            self.frame_t,
            self.raster_line,
            self.raster_x,
            u8::from(self.int_active),
            self.contend_at_pc,
            self.border,
            u8::from(self.ear),
            u8::from(self.beeper),
            r.pc,
            r.sp,
            r.af(),
            r.bc(),
            r.de(),
            r.hl(),
            r.ix(),
            r.iy(),
            u16::from(r.a_) << 8 | u16::from(r.f_),
            u16::from(r.b_) << 8 | u16::from(r.c_),
            u16::from(r.d_) << 8 | u16::from(r.e_),
            u16::from(r.h_) << 8 | u16::from(r.l_),
            r.i,
            r.r,
            r.im,
            r.memptr,
            u8::from(r.iff1),
            u8::from(r.iff2),
            u8::from(r.halted),
            opt_u8(self.paging.page_7ffd),
            opt_u8(self.paging.page_1ffd),
            self.paging.rom_bank,
            opt_u8(self.paging.ram_c000),
            opt_u8(self.paging.screen_bank),
        )
    }
}

fn opt_u8(v: Option<u8>) -> String {
    v.map_or("null".into(), |n| n.to_string())
}

impl Display for Inspect {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        let r = &self.regs;
        writeln!(
            f,
            "model={:?} t={} frame_t={} line={} x={} INT={} contend_pc={}",
            self.model,
            self.cpu_t,
            self.frame_t,
            self.raster_line,
            self.raster_x,
            u8::from(self.int_active),
            self.contend_at_pc
        )?;
        writeln!(
            f,
            "AF={:04X} BC={:04X} DE={:04X} HL={:04X} IX={:04X} IY={:04X} SP={:04X} PC={:04X}",
            r.af(),
            r.bc(),
            r.de(),
            r.hl(),
            r.ix(),
            r.iy(),
            r.sp,
            r.pc
        )?;
        writeln!(
            f,
            "AF'={:04X} BC'={:04X} DE'={:04X} HL'={:04X} I={:02X} R={:02X} IM={} IFF={}/{} HALT={} MEMPTR={:04X}",
            u16::from(r.a_) << 8 | u16::from(r.f_),
            u16::from(r.b_) << 8 | u16::from(r.c_),
            u16::from(r.d_) << 8 | u16::from(r.e_),
            u16::from(r.h_) << 8 | u16::from(r.l_),
            r.i,
            r.r,
            r.im,
            u8::from(r.iff1),
            u8::from(r.iff2),
            u8::from(r.halted),
            r.memptr
        )?;
        writeln!(
            f,
            "border={} ear={} beeper={} 7FFD={:?} 1FFD={:?} ROM={} C000={:?}",
            self.border,
            u8::from(self.ear),
            u8::from(self.beeper),
            self.paging.page_7ffd,
            self.paging.page_1ffd,
            self.paging.rom_bank,
            self.paging.ram_c000
        )?;
        if let Some(t) = &self.tape {
            writeln!(
                f,
                "tape playing={} flash={} speed={}x block={}/{}",
                u8::from(t.playing),
                u8::from(t.flash_load),
                t.speed,
                t.block_index,
                t.block_count
            )?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tape_json_is_valid_object() {
        let t = TapeInspect {
            playing: true,
            flash_load: false,
            speed: 1,
            block_index: 0,
            block_count: 2,
        };
        let s = tape_json(&t);
        assert!(!s.contains('/'), "{s}");
        assert!(s.contains("\"block\":0"));
        assert!(s.contains("\"blocks\":2"));
        assert!(s.starts_with('{') && s.ends_with('}'));
    }
}
