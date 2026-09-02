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
    pub beta: Option<BetaInspect>,
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
    pub experience_load: bool,
    pub speed: u32,
    pub block_index: u32,
    pub block_count: u32,
}

/// Beta Disk / VG93 FDC snapshot (TR-DOS / #140 diagnostics).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BetaInspect {
    pub paged: bool,
    pub track: u8,
    pub sector: u8,
    pub status: u8,
    pub system: u8,
    pub sector_read_count: u32,
    pub cmd_count: u32,
    pub recent_cmds: [u8; 4],
    pub command_ring: Vec<u8>,
}

impl BetaInspect {
    #[must_use]
    pub fn from_disk(b: &bus::BetaDisk) -> Self {
        Self {
            paged: b.paged,
            track: b.track,
            sector: b.sector,
            status: b.status,
            system: b.system,
            sector_read_count: b.sector_read_count,
            cmd_count: b.cmd_count,
            recent_cmds: b.recent_cmds,
            command_ring: b.command_ring().to_vec(),
        }
    }
}

fn beta_inspect_from_48(bus: &bus::Bus48) -> Option<BetaInspect> {
    bus.beta.as_ref().map(BetaInspect::from_disk)
}

fn beta_inspect_from_128(bus: &bus::Bus128) -> Option<BetaInspect> {
    bus.beta.as_ref().map(BetaInspect::from_disk)
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
                let (ay_regs, ay_selected) = if bus.timex_2068 {
                    (Some(bus.ay.regs), Some(bus.ay.selected))
                } else {
                    (None, None)
                };
                Inspect {
                    model: self.model(),
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
                        experience_load: tape_opts.experience_load,
                        speed: tape_opts.speed,
                        block_index: t.block().unwrap_or(0) as u32,
                        block_count: t.block_count() as u32,
                    }),
                    ay_regs,
                    ay_selected,
                    beta: beta_inspect_from_48(bus),
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
                    model: self.model(),
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
                        experience_load: tape_opts.experience_load,
                        speed: tape_opts.speed,
                        block_index: t.block().unwrap_or(0) as u32,
                        block_count: t.block_count() as u32,
                    }),
                    ay_regs: Some(bus.ay.regs),
                    ay_selected: Some(bus.ay.selected),
                    beta: beta_inspect_from_128(bus),
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
                        experience_load: tape_opts.experience_load,
                        speed: tape_opts.speed,
                        block_index: t.block().unwrap_or(0) as u32,
                        block_count: t.block_count() as u32,
                    }),
                    ay_regs: Some(bus.ay.regs),
                    ay_selected: Some(bus.ay.selected),
                    beta: None,
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
        "{{\"playing\":{},\"flash_load\":{},\"experience_load\":{},\"speed\":{},\"block\":{},\"blocks\":{}}}",
        u8::from(t.playing),
        u8::from(t.flash_load),
        u8::from(t.experience_load),
        t.speed,
        t.block_index,
        t.block_count
    )
}

fn beta_json(b: &BetaInspect) -> String {
    let recent: Vec<String> = b.recent_cmds.iter().map(|v| format!("{v}")).collect();
    let ring: Vec<String> = b.command_ring.iter().map(|v| format!("{v}")).collect();
    format!(
        "{{\"paged\":{},\"track\":{},\"sector\":{},\"status\":{},\"system\":{},\
\"sector_read_count\":{},\"cmd_count\":{},\"recent_cmds\":[{}],\"command_ring\":[{}]}}",
        u8::from(b.paged),
        b.track,
        b.sector,
        b.status,
        b.system,
        b.sector_read_count,
        b.cmd_count,
        recent.join(","),
        ring.join(","),
    )
}

impl Inspect {
    /// Hand-rolled JSON (no serde).
    #[must_use]
    pub fn to_json(&self) -> String {
        let r = &self.regs;
        let model = match self.model {
            Model::Spectrum16K => "16k",
            Model::Spectrum48 => "48k",
            Model::Spectrum128 => "128k",
            Model::SpectrumPlus2 => "plus2",
            Model::SpectrumPlus2A => "plus2a",
            Model::SpectrumPlus3 => "plus3",
            Model::Pentagon128 => "pentagon128",
            Model::TimexTC2048 => "timex_tc2048",
            Model::TimexTS2068 => "timex_ts2068",
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
        let beta = self.beta.as_ref().map_or("null".into(), beta_json);
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
\"tape\":{tape},\"ay\":{ay},\"beta\":{beta}\
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
                "tape playing={} flash={} experience={} speed={}x block={}/{}",
                u8::from(t.playing),
                u8::from(t.flash_load),
                u8::from(t.experience_load),
                t.speed,
                t.block_index,
                t.block_count
            )?;
        }
        if let Some(b) = &self.beta {
            writeln!(
                f,
                "beta paged={} track={} sector={} status={:#04x} system={:#04x} sector_reads={} cmds={} recent={:02x?}",
                u8::from(b.paged),
                b.track,
                b.sector,
                b.status,
                b.system,
                b.sector_read_count,
                b.cmd_count,
                b.recent_cmds,
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
            experience_load: false,
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

    #[test]
    fn beta_json_includes_fdc_counters() {
        let b = BetaInspect {
            paged: true,
            track: 1,
            sector: 0,
            status: 0x24,
            system: 0x3c,
            sector_read_count: 2,
            cmd_count: 40,
            recent_cmds: [0x19, 0x19, 0x80, 0x19],
            command_ring: vec![0x19, 0x80],
        };
        let s = beta_json(&b);
        assert!(s.contains("\"paged\":1"));
        assert!(s.contains("\"sector_read_count\":2"));
        assert!(s.contains("\"cmd_count\":40"));
        assert!(s.contains("\"recent_cmds\":[25,25,128,25]"));
        assert!(s.contains("\"command_ring\":[25,128]"));
    }
}
