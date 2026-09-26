//! Instruction-level debugger: pause, PC breakpoints, mem/port watches.

use std::cell::Cell;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};

/// Breakpoint identities stay unique when a host replaces its machine.
static NEXT_PC_HIT_ID: AtomicU64 = AtomicU64::new(0);
const PC_HIT_HISTORY: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PcBreakpointHit {
    /// Process-wide monotonic stop identity.
    pub id: u64,
    /// Program counter where the breakpoint stopped execution.
    pub pc: u16,
}

/// Access watch on a memory address or I/O port.
///
/// Matching uses `(access & mask) == (addr & mask)`. Default `mask = 0xFFFF`
/// is an exact 16-bit match. For Spectrum keyboard row polls (`IN A,(C)` with
/// high-byte row select), watch `addr = 0x00FE` with `mask = 0x00FF` (#387).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Watch {
    pub addr: u16,
    pub read: bool,
    pub write: bool,
    /// Bits compared against the accessed address/port. `0xFFFF` = exact.
    pub mask: u16,
}

impl Watch {
    /// Exact 16-bit address/port watch (`mask = 0xFFFF`).
    #[must_use]
    pub const fn new(addr: u16, read: bool, write: bool) -> Self {
        Self {
            addr,
            read,
            write,
            mask: 0xFFFF,
        }
    }

    /// Masked watch: `(access & mask) == (addr & mask)`.
    #[must_use]
    pub const fn with_mask(addr: u16, mask: u16, read: bool, write: bool) -> Self {
        Self {
            addr,
            read,
            write,
            mask,
        }
    }
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
    pc_hits: VecDeque<PcBreakpointHit>,
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
            pc_hits: VecDeque::new(),
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

    pub fn remove_mem_watch(&mut self, addr: u16) -> bool {
        let before = self.mem_watches.len();
        self.mem_watches.retain(|w| w.addr != addr);
        self.mem_watches.len() < before
    }

    pub fn remove_port_watch(&mut self, addr: u16) -> bool {
        let before = self.port_watches.len();
        self.port_watches.retain(|w| w.addr != addr);
        self.port_watches.len() < before
    }

    pub fn clear_breaks(&mut self) {
        self.pc_breaks.clear();
        self.mem_watches.clear();
        self.port_watches.clear();
        self.last_hit = BreakReason::None;
    }

    /// Recent PC stops, including stops that occurred between observer polls.
    #[must_use]
    /// Return retained PC stops newer than `id`, oldest first.
    pub fn pc_hits_since(&self, id: u64) -> Vec<PcBreakpointHit> {
        self.pc_hits
            .iter()
            .copied()
            .filter(|hit| hit.id > id)
            .collect()
    }

    #[must_use]
    /// Return the newest recorded PC stop identity, or zero when there are none.
    pub fn latest_pc_hit_id(&self) -> u64 {
        self.pc_hits.back().map_or(0, |hit| hit.id)
    }

    #[must_use]
    /// Return the active PC stop when execution is currently paused at one.
    pub fn current_pc_hit(&self) -> Option<PcBreakpointHit> {
        let BreakReason::Pc(pc) = self.last_hit else {
            return None;
        };
        if !self.paused {
            return None;
        }
        self.pc_hits.back().copied().filter(|hit| hit.pc == pc)
    }

    /// A machine reset preserves armed breakpoints but ends the previous stop.
    pub fn reset_stop_state(&mut self) {
        self.paused = false;
        self.last_hit = BreakReason::None;
        self.skip_pc_once = None;
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
            let id = NEXT_PC_HIT_ID
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |old| {
                    old.checked_add(1)
                })
                .expect("PC breakpoint identity space is not exhausted")
                + 1;
            if self.pc_hits.len() == PC_HIT_HISTORY {
                self.pc_hits.pop_front();
            }
            self.pc_hits.push_back(PcBreakpointHit { id, pc });
            return true;
        }
        false
    }

    fn matches(w: Watch, addr: u16, write: bool) -> bool {
        (addr & w.mask) == (w.addr & w.mask) && ((write && w.write) || (!write && w.read))
    }

    fn hit_watch(list: &[Watch], addr: u16, write: bool) -> bool {
        list.iter().copied().any(|w| Self::matches(w, addr, write))
    }

    /// Apply a watch/break hit recorded during `step_once`.
    pub fn apply_hit(&mut self, reason: BreakReason) {
        self.paused = true;
        self.last_hit = reason;
        if let BreakReason::Mem { addr, write, value } = reason {
            if trace::enabled(trace::Category::MEM) {
                trace::emit(trace::EventKind::MemWatch { addr, write, value });
            }
        }
    }

    pub fn on_mem(&mut self, addr: u16, write: bool, value: u8) {
        if Self::hit_watch(&self.mem_watches, addr, write) {
            self.apply_hit(BreakReason::Mem { addr, write, value });
        }
    }

    pub fn on_port(&mut self, port: u16, write: bool, value: u8) {
        if Self::hit_watch(&self.port_watches, port, write) {
            self.apply_hit(BreakReason::Port { port, write, value });
        }
    }
}

/// Shared with `MemIo` adapters for the duration of one CPU step.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pc_hit_ids_advance_across_repeat_stops_and_new_debuggers() {
        let mut debugger = Debugger::new();
        debugger.add_pc_break(0x1234);
        assert!(debugger.check_pc(0x1234));
        let first = debugger.current_pc_hit().expect("first hit");
        debugger.continue_from_pc(0x1234);
        assert!(!debugger.check_pc(0x1234));
        assert!(debugger.check_pc(0x1234));
        let second = debugger.current_pc_hit().expect("repeat hit");
        assert!(second.id > first.id);
        assert_eq!(debugger.pc_hits_since(0), vec![first, second]);

        debugger.reset_stop_state();
        assert!(debugger.current_pc_hit().is_none());
        assert_eq!(debugger.pc_hits_since(0), vec![first, second]);
        assert!(debugger.check_pc(0x1234));
        let after_reset = debugger.current_pc_hit().expect("post-reset hit");
        assert!(after_reset.id > second.id);

        let mut replacement = Debugger::new();
        replacement.add_pc_break(0x1234);
        assert!(replacement.check_pc(0x1234));
        assert!(replacement.current_pc_hit().expect("new machine hit").id > after_reset.id);
    }

    #[test]
    fn exact_mask_requires_full_port_match() {
        let w = Watch::new(0x00FE, true, false);
        assert!(Debugger::matches(w, 0x00FE, false));
        assert!(!Debugger::matches(w, 0x3CFE, false));
    }

    #[test]
    fn low_byte_mask_matches_keyboard_row_ports() {
        let w = Watch::with_mask(0x00FE, 0x00FF, true, false);
        assert!(Debugger::matches(w, 0x00FE, false));
        assert!(Debugger::matches(w, 0x3CFE, false));
        assert!(Debugger::matches(w, 0x7FFE, false));
        assert!(!Debugger::matches(w, 0x00FF, false));
        assert!(!Debugger::matches(w, 0x3CFE, true)); // write not enabled
    }
}
