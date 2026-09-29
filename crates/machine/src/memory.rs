//! CPU memory and I/O adapters with model-specific ULA timing.

use std::cell::Cell;

use bus::{Bus128, Bus48, BusPlus3};
use ula::{FRAME_TSTATES_128, FRAME_TSTATES_48, FRAME_TSTATES_PENTAGON};
use z80::{Io, Memory};

use crate::{debugger, BreakReason, Debugger};

/// Memory+Io adapter for 48K.
#[derive(Debug)]
pub struct MemIo48<'a> {
    pub bus: &'a mut Bus48,
    pub(crate) watch: Option<debugger::WatchHook<'a>>,
    /// `cpu.t` at the start of the current instruction (for mid-instruction ULA time).
    pub(crate) t_step_start: u64,
    /// When set, the first `read` at this PC runs IF1 pre/post opcode-fetch paging.
    pub(crate) opcode_pc: Option<u16>,
}

impl MemIo48<'_> {
    #[inline]
    fn ula_t(&self, t: u64) -> u32 {
        let dt = t.wrapping_sub(self.t_step_start) as u32;
        (self.bus.frame_t.wrapping_add(dt)) % FRAME_TSTATES_48
    }
}

impl Memory for MemIo48<'_> {
    fn read(&mut self, addr: u16, t: u64) -> (u8, u32) {
        let mut unpage_after = false;
        let is_opcode = self.opcode_pc == Some(addr);
        if is_opcode {
            if let Some(if1) = self.bus.interface1.as_mut() {
                if1.pre_opcode_fetch(addr);
                unpage_after = addr == 0x0700;
            }
            self.opcode_pc = None;
        }
        let wait = if Bus48::is_contended(addr) {
            ula::contention_delay_48(self.ula_t(t))
        } else {
            0
        };
        if wait > 0 && trace::enabled(trace::Category::BUS) {
            emit_contend_sampled(addr, self.ula_t(t), wait);
        }
        let v = self.bus.read(addr);
        if unpage_after {
            if let Some(if1) = self.bus.interface1.as_mut() {
                if1.post_opcode_fetch(0x0700);
            }
        }
        if let Some(w) = self.watch.as_ref() {
            w.mem_access(addr, false, v);
        }
        (v, wait)
    }

    fn write(&mut self, addr: u16, value: u8, t: u64) -> u32 {
        let wait = if Bus48::is_contended(addr) {
            ula::contention_delay_48(self.ula_t(t))
        } else {
            0
        };
        if wait > 0 && trace::enabled(trace::Category::BUS) {
            emit_contend_sampled(addr, self.ula_t(t), wait);
        }
        self.bus.write(addr, value);
        if let Some(w) = self.watch.as_ref() {
            w.mem_access(addr, true, value);
        }
        wait
    }

    fn m1_refresh(&mut self, refresh_addr: u16, t: u64, m1_contended: bool) {
        self.bus.divmmc_after_m1_refresh();
        let i = (refresh_addr >> 8) as u8;
        let r = (refresh_addr & 0x7f) as u8;
        let frame_t = self.ula_t(t);
        let screen = &self.bus.ram[..6912];
        let ovs = ula::snow_overrides(
            frame_t,
            r,
            m1_contended,
            ula::snow_possible(i),
            ula::SnowTiming::Class48,
            screen,
            None,
        );
        for o in ovs {
            self.bus.ula.record_snow(o.line, o.col, o.kind, o.byte);
        }
    }
}

impl Io for MemIo48<'_> {
    fn in_port(&mut self, port: u16, t: u64) -> (u8, u32) {
        let ft = self.ula_t(t);
        let wait = ula::io_contention_extra_48(ft, port);
        // Z80 latches the data bus on the last T of the I/O cycle. Odd ports are
        // `N:4` (no I/O wait); even ports add FAQ waits before that last T.
        let sample = ft.wrapping_add(3).wrapping_add(wait) % FRAME_TSTATES_48;
        let saved = self.bus.frame_t;
        self.bus.frame_t = sample;
        let v = self.bus.in_port(port);
        self.bus.frame_t = saved;
        if let Some(w) = self.watch.as_ref() {
            w.port_access(port, false, v);
        }
        (v, wait)
    }

    fn out_port(&mut self, port: u16, value: u8, t: u64) -> u32 {
        let ft = self.ula_t(t);
        let wait = ula::io_contention_extra_48(ft, port);
        let saved = self.bus.frame_t;
        self.bus.frame_t = ft;
        self.bus.out_port(port, value);
        self.bus.frame_t = saved;
        if let Some(w) = self.watch.as_ref() {
            w.port_access(port, true, value);
        }
        wait
    }
}

#[derive(Debug)]
pub struct MemIo128<'a> {
    pub bus: &'a mut Bus128,
    pub(crate) watch: Option<debugger::WatchHook<'a>>,
    pub(crate) t_step_start: u64,
    /// When set, the first `read` at this PC runs IF1 pre/post opcode-fetch paging.
    pub(crate) opcode_pc: Option<u16>,
    /// Pentagon 128: 71680 T/frame, no memory or I/O contention (no ULA snow).
    /// Also true for Scorpion ZS-256 (same clone ULA class).
    pub(crate) pentagon: bool,
}

impl MemIo128<'_> {
    #[inline]
    fn frame_len(&self) -> u32 {
        if self.pentagon || self.bus.scorpion {
            FRAME_TSTATES_PENTAGON
        } else {
            FRAME_TSTATES_128
        }
    }

    #[inline]
    fn ula_t(&self, t: u64) -> u32 {
        let dt = t.wrapping_sub(self.t_step_start) as u32;
        (self.bus.frame_t.wrapping_add(dt)) % self.frame_len()
    }

    #[inline]
    fn no_contend(&self) -> bool {
        self.pentagon || self.bus.scorpion
    }
}

impl Memory for MemIo128<'_> {
    fn read(&mut self, addr: u16, t: u64) -> (u8, u32) {
        let mut unpage_after = false;
        let is_opcode = self.opcode_pc == Some(addr);
        if is_opcode {
            if let Some(if1) = self.bus.interface1.as_mut() {
                if1.pre_opcode_fetch(addr);
                unpage_after = addr == 0x0700;
            }
            self.opcode_pc = None;
        }
        let ft = self.ula_t(t);
        let saved = self.bus.frame_t;
        self.bus.frame_t = ft;
        let wait = if self.no_contend() {
            0
        } else {
            self.bus.contend_at(addr)
        };
        self.bus.frame_t = saved;
        if wait > 0 && trace::enabled(trace::Category::BUS) {
            emit_contend_sampled(addr, ft, wait);
        }
        let v = self.bus.read(addr);
        if unpage_after {
            if let Some(if1) = self.bus.interface1.as_mut() {
                if1.post_opcode_fetch(0x0700);
            }
        }
        if let Some(w) = self.watch.as_ref() {
            w.mem_access(addr, false, v);
        }
        (v, wait)
    }

    fn write(&mut self, addr: u16, value: u8, t: u64) -> u32 {
        let ft = self.ula_t(t);
        let saved = self.bus.frame_t;
        self.bus.frame_t = ft;
        let wait = if self.no_contend() {
            0
        } else {
            self.bus.contend_at(addr)
        };
        self.bus.frame_t = saved;
        if wait > 0 && trace::enabled(trace::Category::BUS) {
            emit_contend_sampled(addr, ft, wait);
        }
        self.bus.write(addr, value);
        if let Some(w) = self.watch.as_ref() {
            w.mem_access(addr, true, value);
        }
        wait
    }

    fn m1_refresh(&mut self, refresh_addr: u16, t: u64, m1_contended: bool) {
        // DivMMC delayed automap commits on every M1 refresh (before snow).
        self.bus.divmmc_after_m1_refresh();
        // Pentagon / Scorpion and Amstrad +2A/+3 paths omit snow — no original-ULA snow.
        if self.no_contend() {
            return;
        }
        let i = (refresh_addr >> 8) as u8;
        let r = (refresh_addr & 0x7f) as u8;
        let frame_t = self.ula_t(t);
        let c000_contended = self.bus.c000_contended();
        if !ula::snow_possible_128(i, c000_contended) {
            return;
        }
        let screen_bank = if self.bus.page & 0x08 != 0 { 7 } else { 5 };
        let c000_bank = usize::from(self.bus.page & 7);
        let Some(i_bank) = ula::i_pointed_bank_128(i, c000_bank) else {
            return;
        };
        let src_bank = ula::snow_source_bank_128(i_bank, screen_bank);
        // Borrow banks by index (shared refs) — never allocate 6912 bytes in the M1 hot path.
        let corrupt_source = (src_bank != screen_bank).then(|| &self.bus.banks[src_bank][..6912]);
        let screen = &self.bus.banks[screen_bank][..6912];
        let ovs = ula::snow_overrides(
            frame_t,
            r,
            m1_contended,
            true,
            ula::SnowTiming::Class128,
            screen,
            corrupt_source,
        );
        for o in ovs {
            self.bus.ula.record_snow(o.line, o.col, o.kind, o.byte);
        }
    }
}

impl Io for MemIo128<'_> {
    fn in_port(&mut self, port: u16, t: u64) -> (u8, u32) {
        let ft = self.ula_t(t);
        let wait = if self.no_contend() {
            0
        } else {
            ula::io_contention_extra_128(ft, port, self.bus.c000_contended())
        };
        let sample = ft.wrapping_add(3).wrapping_add(wait) % self.frame_len();
        let saved = self.bus.frame_t;
        self.bus.frame_t = sample;
        let v = self.bus.in_port(port);
        self.bus.frame_t = saved;
        if let Some(w) = self.watch.as_ref() {
            w.port_access(port, false, v);
        }
        (v, wait)
    }

    fn out_port(&mut self, port: u16, value: u8, t: u64) -> u32 {
        let ft = self.ula_t(t);
        let wait = if self.no_contend() {
            0
        } else {
            ula::io_contention_extra_128(ft, port, self.bus.c000_contended())
        };
        let saved = self.bus.frame_t;
        self.bus.frame_t = ft;
        self.bus.out_port(port, value);
        self.bus.frame_t = saved;
        if let Some(w) = self.watch.as_ref() {
            w.port_access(port, true, value);
        }
        wait
    }
}

#[derive(Debug)]
/// Amstrad +2A/+3 memory+I/O — **no** original-ULA snow (`m1_refresh` stays default no-op).
pub struct MemIoPlus3<'a> {
    pub bus: &'a mut BusPlus3,
    pub(crate) watch: Option<debugger::WatchHook<'a>>,
    pub(crate) t_step_start: u64,
}

impl MemIoPlus3<'_> {
    #[inline]
    fn ula_t(&self, t: u64) -> u32 {
        let dt = t.wrapping_sub(self.t_step_start) as u32;
        (self.bus.frame_t.wrapping_add(dt)) % FRAME_TSTATES_128
    }
}

impl Memory for MemIoPlus3<'_> {
    fn read(&mut self, addr: u16, t: u64) -> (u8, u32) {
        let ft = self.ula_t(t);
        let saved = self.bus.frame_t;
        self.bus.frame_t = ft;
        let wait = self.bus.contend_at(addr);
        self.bus.frame_t = saved;
        if wait > 0 && trace::enabled(trace::Category::BUS) {
            emit_contend_sampled(addr, ft, wait);
        }
        let v = self.bus.read(addr);
        if let Some(w) = self.watch.as_ref() {
            w.mem_access(addr, false, v);
        }
        (v, wait)
    }

    fn write(&mut self, addr: u16, value: u8, t: u64) -> u32 {
        let ft = self.ula_t(t);
        let saved = self.bus.frame_t;
        self.bus.frame_t = ft;
        let wait = self.bus.contend_at(addr);
        self.bus.frame_t = saved;
        if wait > 0 && trace::enabled(trace::Category::BUS) {
            emit_contend_sampled(addr, ft, wait);
        }
        self.bus.write(addr, value);
        if let Some(w) = self.watch.as_ref() {
            w.mem_access(addr, true, value);
        }
        wait
    }
}

impl Io for MemIoPlus3<'_> {
    fn in_port(&mut self, port: u16, t: u64) -> (u8, u32) {
        // +2A/+3 gate array: no Sinclair-style ULA I/O contention. FDC ports
        // `2FFD`/`3FFD` are on the gate array as well (wait=0). Confirmed for
        // +3DOS; no accuracy follow-up from #141.
        let _ = (port, t);
        let v = self.bus.in_port(port);
        if let Some(w) = self.watch.as_ref() {
            w.port_access(port, false, v);
        }
        (v, 0)
    }

    fn out_port(&mut self, port: u16, value: u8, t: u64) -> u32 {
        let _ = t;
        self.bus.out_port(port, value);
        if let Some(w) = self.watch.as_ref() {
            w.port_access(port, true, value);
        }
        0
    }
}

pub(super) fn emit_contend_sampled(addr: u16, frame_t: u32, wait: u32) {
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    if n.is_multiple_of(64) {
        trace::emit(trace::EventKind::BusContend {
            addr,
            frame_t,
            wait,
        });
    }
}

pub(super) fn mem_port_watch<'a>(
    debugger: &'a Debugger,
    hit: &'a Cell<Option<BreakReason>>,
) -> Option<debugger::WatchHook<'a>> {
    if debugger.mem_watches.is_empty() && debugger.port_watches.is_empty() {
        None
    } else {
        Some(debugger::WatchHook {
            mem: &debugger.mem_watches,
            port: &debugger.port_watches,
            hit,
        })
    }
}
