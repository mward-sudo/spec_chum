//! Instruction-level debugger: pause, PC breakpoints, mem/port watches.

use std::cell::Cell;

/// Access watch on a memory address or I/O port.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Watch {
    pub addr: u16,
    pub read: bool,
    pub write: bool,
}

/// Why execution stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BreakReason {
    None,
    Pc(u16),
    Mem { addr: u16, write: bool, value: u8 },
    Port { port: u16, write: bool, value: u8 },
    Halt,
    Budget,
}

impl BreakReason {
    #[must_use]
    pub fn is_stop(self) -> bool {
        !matches!(self, Self::None)
    }
}

/// Per-machine debugger (inactive until armed).
#[derive(Clone, Debug)]
pub struct Debugger {
    pub paused: bool,
    pub pc_breaks: Vec<u16>,
    pub mem_watches: Vec<Watch>,
    pub port_watches: Vec<Watch>,
    pub last_hit: BreakReason,
    /// Skip one PC-break at this address after Continue/Step from that break.
    skip_pc_once: Option<u16>,
}

impl Default for Debugger {
    fn default() -> Self {
        Self {
            paused: false,
            pc_breaks: Vec::new(),
            mem_watches: Vec::new(),
            port_watches: Vec::new(),
            last_hit: BreakReason::None,
            skip_pc_once: None,
        }
    }
}

impl Debugger {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn armed(&self) -> bool {
        !self.pc_breaks.is_empty() || !self.mem_watches.is_empty() || !self.port_watches.is_empty()
    }

    pub fn add_pc_break(&mut self, pc: u16) {
        if !self.pc_breaks.contains(&pc) {
            self.pc_breaks.push(pc);
        }
    }

    pub fn remove_pc_break(&mut self, pc: u16) {
        self.pc_breaks.retain(|&p| p != pc);
    }

    pub fn add_mem_watch(&mut self, w: Watch) {
        self.mem_watches.push(w);
    }

    pub fn add_port_watch(&mut self, w: Watch) {
        self.port_watches.push(w);
    }

    pub fn clear_breaks(&mut self) {
        self.pc_breaks.clear();
        self.mem_watches.clear();
        self.port_watches.clear();
        self.last_hit = BreakReason::None;
    }

    /// Continue from a PC breakpoint without immediately re-hitting it.
    pub fn continue_from_pc(&mut self, pc: u16) {
        self.paused = false;
        self.skip_pc_once = Some(pc);
        self.last_hit = BreakReason::None;
    }

    /// Returns true if this instruction should not run (paused at breakpoint).
    pub fn check_pc(&mut self, pc: u16) -> bool {
        if self.skip_pc_once == Some(pc) {
            self.skip_pc_once = None;
            return false;
        }
        if self.pc_breaks.contains(&pc) {
            self.paused = true;
            self.last_hit = BreakReason::Pc(pc);
            return true;
        }
        false
    }

    fn hit_watch(list: &[Watch], addr: u16, write: bool) -> bool {
        list.iter()
            .any(|w| w.addr == addr && ((write && w.write) || (!write && w.read)))
    }

    pub fn on_mem(&mut self, addr: u16, write: bool, value: u8) {
        if Self::hit_watch(&self.mem_watches, addr, write) {
            self.paused = true;
            self.last_hit = BreakReason::Mem { addr, write, value };
            if trace::enabled(trace::Category::MEM) {
                trace::emit(trace::EventKind::MemWatch { addr, write, value });
            }
        }
    }

    pub fn on_port(&mut self, port: u16, write: bool, value: u8) {
        if Self::hit_watch(&self.port_watches, port, write) {
            self.paused = true;
            self.last_hit = BreakReason::Port { port, write, value };
        }
    }
}

/// Shared with MemIo adapters for the duration of one CPU step.
#[derive(Debug)]
pub(crate) struct WatchHook<'a> {
    pub mem: &'a [Watch],
    pub port: &'a [Watch],
    pub hit: &'a Cell<Option<BreakReason>>,
}

impl WatchHook<'_> {
    pub(crate) fn mem_access(&self, addr: u16, write: bool, value: u8) {
        if Debugger::hit_watch(self.mem, addr, write) {
            self.hit.set(Some(BreakReason::Mem { addr, write, value }));
        }
    }

    pub(crate) fn port_access(&self, port: u16, write: bool, value: u8) {
        if Debugger::hit_watch(self.port, port, write) {
            self.hit.set(Some(BreakReason::Port { port, write, value }));
        }
    }
}
