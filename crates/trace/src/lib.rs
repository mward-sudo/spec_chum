//! Structured emulator tracing: category-gated ring buffer with dump API.
//!
//! Hot path: when no categories are enabled, [`emit`] / [`enabled`] are a single
//! `AtomicU64` load (`Relaxed`) and return — no allocation and no lock.
//!
//! Layout (#409 cohesion split): [`category`], [`event`], [`dump`]. Public API
//! surface is unchanged.

mod category;
mod dump;
mod event;

#[cfg(test)]
mod tests;

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

pub use category::Category;
pub use dump::{
    dump_filtered, dump_json, dump_ndjson, dump_string, dump_to_env_file, dump_to_file,
    dump_to_stderr, dump_to_writer, flush_append, DumpFilter,
};
pub use event::{EventKind, FlashSkipReason, RegSnap, TraceEvent};

#[cfg(test)]
pub use dump::{configure_append_file_for_tests, reset_append_sink_for_tests};

/// Default ring capacity (overridable via `SPEC_CHUM_TRACE_CAPACITY`).
pub const DEFAULT_CAPACITY: usize = 8192;

use dump::maybe_append;

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
    Category::from_bits(ENABLED.load(Ordering::Relaxed))
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
    ring().lock().map_or(0, |g| g.events.len())
}

/// Exclusive lock for tests that mutate the global ring (avoids cross-test races).
pub fn test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
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
